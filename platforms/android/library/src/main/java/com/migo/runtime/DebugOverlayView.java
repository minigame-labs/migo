package com.migo.runtime;

import android.content.Context;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Debug;
import android.os.Handler;
import android.os.Looper;
import android.os.Process;
import android.os.SystemClock;
import android.util.TypedValue;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.migo.runtime.internal.NativeBridge;

import java.io.BufferedReader;
import java.io.FileReader;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Semi-transparent debug overlay that displays real-time engine statistics.
 *
 * <h3>Layout</h3>
 * <pre>
 *  59.8 FPS  16.7ms  D:0
 *  Startup: 342ms  1st: 156ms
 *  Mem: 87MB (N:62 J:25)
 *  CPU: 12.3%
 * </pre>
 *
 * @since 1.0.0
 */
public class DebugOverlayView extends FrameLayout {

    private static final int BG_COLOR = 0xCC1A1A2E;
    private static final int TEXT_COLOR = 0xFFE0E0E0;
    private static final int ACCENT_COLOR = 0xFF4FC3F7;  // light blue for labels
    private static final int WARN_COLOR = 0xFFFF9800;
    private static final float TEXT_SIZE_SP = 10.5f;
    private static final int CORNER_RADIUS_DP = 6;
    private static final int PADDING_H_DP = 8;
    private static final int PADDING_V_DP = 5;
    private static final int DEFAULT_UPDATE_INTERVAL_MS = 500;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final int sessionId;

    // Row TextViews
    private final TextView rowFps;
    private final TextView rowTiming;
    private final TextView rowMem;
    private final TextView rowCpu;
    private TextView rowFatal;

    private Runnable updateRunnable;
    private int updateIntervalMs = DEFAULT_UPDATE_INTERVAL_MS;

    private volatile long startupTimeMs = -1;
    private boolean firstRenderShown = false;
    private long firstRenderMs = 0;

    // CPU delta tracking
    private long prevCpuTime = -1;
    private long prevUptime = -1;

    public DebugOverlayView(Context context, int sessionId) {
        super(context);
        this.sessionId = sessionId;

        LinearLayout container = new LinearLayout(context);
        container.setOrientation(LinearLayout.VERTICAL);

        int padH = dpToPx(PADDING_H_DP);
        int padV = dpToPx(PADDING_V_DP);
        container.setPadding(padH, padV, padH, padV);

        GradientDrawable bg = new GradientDrawable();
        bg.setColor(BG_COLOR);
        bg.setCornerRadius(dpToPx(CORNER_RADIUS_DP));
        container.setBackground(bg);

        rowFps = createRow(container, "-- FPS  --ms");
        rowTiming = createRow(container, "Startup: --  1st: --");
        rowMem = createRow(container, "Mem: --");
        rowCpu = createRow(container, "CPU: --");

        addView(container);
    }

    private TextView createRow(LinearLayout parent, String text) {
        TextView tv = new TextView(getContext());
        tv.setTypeface(Typeface.MONOSPACE);
        tv.setTextColor(TEXT_COLOR);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SIZE_SP);
        tv.setText(text);
        parent.addView(tv);
        return tv;
    }

    /**
     * Set the startup time (session creation to modMain return).
     */
    void setStartupTimeMs(long ms) {
        this.startupTimeMs = ms;
        handler.post(new Runnable() {
            @Override
            public void run() {
                updateTimingRow();
            }
        });
    }

    public void setUpdateInterval(int intervalMs) {
        this.updateIntervalMs = Math.max(100, intervalMs);
    }

    public void startMonitoring() {
        stopMonitoring();
        updateRunnable = new Runnable() {
            @Override
            public void run() {
                refreshStats();
                handler.postDelayed(this, updateIntervalMs);
            }
        };
        handler.post(updateRunnable);
    }

    public void stopMonitoring() {
        if (updateRunnable != null) {
            handler.removeCallbacks(updateRunnable);
            updateRunnable = null;
        }
    }

    private void refreshStats() {
        byte[] data = NativeBridge.getDebugStats(sessionId);
        if (data == null || data.length < 12) return;

        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        int fpsX10 = buf.getInt(0);
        int frameTimeUs = buf.getInt(4);
        int dropped = buf.getInt(8);
        int fatalError = data.length >= 16 ? buf.getInt(12) : 0;
        int firstFrameMs = data.length >= 20 ? buf.getInt(16) : 0;

        float fps = (fpsX10 & 0xFFFFFFFFL) / 10f;
        float frameMs = (frameTimeUs & 0xFFFFFFFFL) / 1000f;

        // Row 1: FPS + frame time + dropped (compact)
        StringBuilder sb = new StringBuilder();
        sb.append(String.format("%.1f FPS  %.1fms", fps, frameMs));
        if (dropped > 0) {
            sb.append(String.format("  D:%d", dropped & 0xFFFFFFFFL));
        }
        rowFps.setText(sb.toString());
        rowFps.setTextColor(fps < 25 ? WARN_COLOR : TEXT_COLOR);

        // First render (one-shot)
        if (!firstRenderShown && firstFrameMs > 0) {
            firstRenderShown = true;
            firstRenderMs = firstFrameMs & 0xFFFFFFFFL;
            updateTimingRow();
        }

        // Row 3: Memory
        refreshMemory();

        // Row 4: CPU
        refreshCpu();

        // Fatal error (extra row, only if needed)
        if (fatalError != 0) {
            if (rowFatal == null) {
                rowFatal = new TextView(getContext());
                rowFatal.setTypeface(Typeface.MONOSPACE);
                rowFatal.setTextColor(WARN_COLOR);
                rowFatal.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SIZE_SP);
                ((LinearLayout) rowFps.getParent()).addView(rowFatal);
            }
            rowFatal.setText("FATAL: " + fatalError);
        }
    }

    private void updateTimingRow() {
        String startup = startupTimeMs >= 0
                ? String.format("%dms", startupTimeMs)
                : "--";
        String firstRender = firstRenderShown
                ? String.format("%dms", firstRenderMs)
                : "--";
        rowTiming.setText(String.format("Startup: %s  1st: %s", startup, firstRender));
    }

    private void refreshMemory() {
        long nativeMB = Debug.getNativeHeapAllocatedSize() / (1024 * 1024);
        Runtime rt = Runtime.getRuntime();
        long javaMB = (rt.totalMemory() - rt.freeMemory()) / (1024 * 1024);
        long totalMB = nativeMB + javaMB;
        rowMem.setText(String.format("Mem: %dMB (N:%d J:%d)", totalMB, nativeMB, javaMB));
    }

    private void refreshCpu() {
        try {
            int pid = Process.myPid();
            BufferedReader reader = new BufferedReader(new FileReader("/proc/" + pid + "/stat"));
            String line = reader.readLine();
            reader.close();
            if (line == null) return;

            int commEnd = line.lastIndexOf(')');
            if (commEnd < 0) return;
            String[] fields = line.substring(commEnd + 2).trim().split("\\s+");
            if (fields.length < 13) return;
            long utime = Long.parseLong(fields[11]);
            long stime = Long.parseLong(fields[12]);
            long cpuTime = utime + stime;
            long uptimeMs = SystemClock.elapsedRealtime();

            if (prevCpuTime >= 0 && prevUptime >= 0) {
                long deltaCpu = cpuTime - prevCpuTime;
                long deltaUptime = uptimeMs - prevUptime;
                if (deltaUptime > 0) {
                    // HZ=100 assumed, 1 jiffy = 10ms
                    float cpuPercent = (deltaCpu * 10.0f * 100.0f) / deltaUptime;
                    cpuPercent = Math.min(cpuPercent, 999.9f);
                    rowCpu.setText(String.format("CPU: %.1f%%", cpuPercent));
                    rowCpu.setTextColor(cpuPercent > 80 ? WARN_COLOR : TEXT_COLOR);
                }
            }
            prevCpuTime = cpuTime;
            prevUptime = uptimeMs;
        } catch (Exception e) {
            // CPU stats not available on this device
        }
    }

    private int dpToPx(int dp) {
        return (int) (dp * getContext().getResources().getDisplayMetrics().density + 0.5f);
    }

    @Override
    protected void onDetachedFromWindow() {
        super.onDetachedFromWindow();
        stopMonitoring();
    }
}
