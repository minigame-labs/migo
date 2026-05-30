package com.migo.runtime;

import android.content.Context;
import android.graphics.PixelFormat;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Debug;
import android.os.Handler;
import android.os.Looper;
import android.os.Process;
import android.os.SystemClock;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.migo.runtime.internal.NativeBridge;

import java.io.BufferedReader;
import java.io.FileReader;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Semi-transparent debug overlay that displays real-time engine statistics.
 * <p>
 * Uses a separate {@link WindowManager} panel so it is always rendered above
 * the game SurfaceView, regardless of Z-order or elevation.
 *
 */
public class DebugOverlayView extends LinearLayout {

    private static final int STATS_MAGIC      = 0x4D47; // 'M' 'G'
    private static final int STATS_HEADER_LEN = 4;      // 2 magic + 2 version

    private static final int BG_COLOR      = 0xCC1B1B1B; // 80% dark
    private static final int TEXT_COLOR     = 0xFFE0E0E0; // Grey 300
    private static final int LABEL_COLOR   = 0xFF90A4AE; // Blue Grey 300
    private static final int ACCENT_COLOR  = 0xFF64B5F6; // Blue 300
    private static final int WARN_COLOR    = 0xFFFFB74D; // Orange 300
    private static final int ERROR_COLOR   = 0xFFEF5350; // Red 400

    private static final float TEXT_SIZE_SP   = 11f;
    private static final int CORNER_RADIUS_DP = 6;
    private static final int PADDING_H_DP     = 10;
    private static final int PADDING_V_DP     = 6;
    private static final int ROW_SPACING_DP   = 1;
    private static final int DEFAULT_UPDATE_INTERVAL_MS = 500;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final int sessionId;

    // Row TextViews
    private final TextView rowFps;
    private final TextView rowTiming;
    private final TextView rowRender;
    private final TextView rowQueue;
    private final TextView rowSnap;
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

    // Drag support — all coordinates in screen space
    private float downRawX, downRawY;
    private int downX, downY;          // WindowManager LayoutParams x/y at touch-down
    private boolean isDragging = false;
    private int touchSlop;

    // WindowManager panel hosting
    private WindowManager wm;
    private WindowManager.LayoutParams wmParams;
    private boolean attached = false;

    public DebugOverlayView(Context context, int sessionId) {
        super(context);
        this.sessionId = sessionId;
        this.touchSlop = ViewConfiguration.get(context).getScaledTouchSlop();
        setOrientation(VERTICAL);

        int padH = dpToPx(PADDING_H_DP);
        int padV = dpToPx(PADDING_V_DP);
        setPadding(padH, padV, padH, padV);

        GradientDrawable bg = new GradientDrawable();
        bg.setColor(BG_COLOR);
        bg.setCornerRadius(dpToPx(CORNER_RADIUS_DP));
        setBackground(bg);

        rowFps    = createRow("-- FPS  --ms");
        rowRender = createRow("RAF: --  Swap: --  UQ: --  GM: --");
        rowQueue  = createRow("Q: --  PB: --  WR: --  SK: --  DU: --");
        rowSnap   = createRow("Snap: --  Up: --  FB: --  FR: --");
        rowCpu    = createRow("CPU: --");
        rowMem    = createRow("Mem: --");
        rowTiming = createRow("Start: --  1st: --");
    }

    // ==================== WindowManager hosting ====================

    /**
     * Attach this view as a sub-panel window so it always floats above the
     * SurfaceView used for game rendering.
     *
     * @param anchorToken window token of the host Activity's decor view
     */
    public void attachToWindow(View anchor) {
        if (attached) return;
        wm = (WindowManager) getContext().getSystemService(Context.WINDOW_SERVICE);
        if (wm == null) return;

        wmParams = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_APPLICATION_PANEL,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
                PixelFormat.TRANSLUCENT);
        wmParams.gravity = Gravity.TOP | Gravity.START;
        wmParams.token = anchor.getWindowToken();
        // Initial position: top-end with margin
        wmParams.x = anchor.getWidth() - dpToPx(160);
        wmParams.y = dpToPx(32);

        wm.addView(this, wmParams);
        attached = true;
    }

    /**
     * Remove from WindowManager (call before Activity finishes).
     */
    public void detachFromWindow() {
        if (!attached) return;
        attached = false;
        stopMonitoring();
        handler.removeCallbacksAndMessages(null);
        try {
            wm.removeViewImmediate(this);
        } catch (Exception ignored) {
        }
    }

    // ==================== Touch / Drag ====================

    @Override
    public boolean onInterceptTouchEvent(MotionEvent ev) {
        // Intercept all touches so child TextViews don't consume them
        return true;
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        if (wmParams == null) return super.onTouchEvent(event);

        switch (event.getAction()) {
            case MotionEvent.ACTION_DOWN:
                downRawX = event.getRawX();
                downRawY = event.getRawY();
                downX = wmParams.x;
                downY = wmParams.y;
                isDragging = false;
                return true;

            case MotionEvent.ACTION_MOVE: {
                float dx = event.getRawX() - downRawX;
                float dy = event.getRawY() - downRawY;
                if (!isDragging) {
                    if (Math.abs(dx) > touchSlop || Math.abs(dy) > touchSlop) {
                        isDragging = true;
                    }
                }
                if (isDragging) {
                    wmParams.x = downX + (int) dx;
                    wmParams.y = downY + (int) dy;
                    wm.updateViewLayout(this, wmParams);
                }
                return true;
            }

            case MotionEvent.ACTION_UP:
                if (!isDragging) {
                    performClick();
                }
                isDragging = false;
                return true;

            default:
                return super.onTouchEvent(event);
        }
    }

    @Override
    public boolean performClick() {
        return super.performClick();
    }

    // ==================== Row helpers ====================

    private TextView createRow(String text) {
        TextView tv = new TextView(getContext());
        tv.setTypeface(Typeface.MONOSPACE);
        tv.setTextColor(TEXT_COLOR);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SIZE_SP);
        tv.setText(text);
        tv.setIncludeFontPadding(false);

        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.topMargin = dpToPx(ROW_SPACING_DP);
        tv.setLayoutParams(lp);

        addView(tv);
        return tv;
    }

    // ==================== Data ====================

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
        if (data == null || data.length < STATS_HEADER_LEN + 12) return;

        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);

        // Validate magic header to detect Rust/Java protocol mismatch.
        int magic = buf.getShort(0) & 0xFFFF;
        if (magic != STATS_MAGIC) return;

        int h = STATS_HEADER_LEN; // payload starts at byte 4
        int fpsX10      = buf.getInt(h + 0);
        int frameTimeUs = buf.getInt(h + 4);
        int dropped     = buf.getInt(h + 8);
        int fatalError  = data.length >= h + 16 ? buf.getInt(h + 12) : 0;
        int firstFrameMs = data.length >= h + 20 ? buf.getInt(h + 16) : 0;
        int cmdDrops     = data.length >= h + 24 ? buf.getInt(h + 20) : 0;
        int rafLatencyUs = data.length >= h + 28 ? buf.getInt(h + 24) : 0;
        int swapBlockUs = data.length >= h + 32 ? buf.getInt(h + 28) : 0;
        int uploadQueueDepth = data.length >= h + 36 ? buf.getInt(h + 32) : 0;
        int glyphAtlasMiss = data.length >= h + 40 ? buf.getInt(h + 36) : 0;
        // Render optimization metrics (appended at payload byte 40+).
        int partialDamageFrames = data.length >= h + 44 ? buf.getInt(h + 40) : 0;
        int fullSurfaceFrames   = data.length >= h + 48 ? buf.getInt(h + 44) : 0;
        int damageAreaKpx       = data.length >= h + 52 ? buf.getInt(h + 48) : 0;
        int uploadFrameReject   = data.length >= h + 56 ? buf.getInt(h + 52) : 0;
        int droppedUploadRecov  = data.length >= h + 60 ? buf.getInt(h + 56) : 0;
        // v4 queue / cache observability at payload offsets 92..112.
        // Tail-append only; a v3 native (payload 92) will simply
        // short-circuit these reads to 0 via the length guards.
        int renderQueueLen      = data.length >= h + 96  ? buf.getInt(h + 92)  : 0;
        int collectorPending    = data.length >= h + 100 ? buf.getInt(h + 96)  : 0;
        int webglErrOverflow    = data.length >= h + 104 ? buf.getInt(h + 100) : 0;
        int skImageWrappers     = data.length >= h + 108 ? buf.getInt(h + 104) : 0;
        int deferredUploads     = data.length >= h + 112 ? buf.getInt(h + 108) : 0;
        // v5 Canvas2D zero-readback snapshot counters at payload
        // offsets 112..128.  Older natives short-circuit to 0.
        int snapTaken           = data.length >= h + 116 ? buf.getInt(h + 112) : 0;
        int snapFallback        = data.length >= h + 120 ? buf.getInt(h + 116) : 0;
        int snapUpload          = data.length >= h + 124 ? buf.getInt(h + 120) : 0;
        int snapForcedReadback  = data.length >= h + 128 ? buf.getInt(h + 124) : 0;

        float fps     = (fpsX10 & 0xFFFFFFFFL) / 10f;
        float frameMs = (frameTimeUs & 0xFFFFFFFFL) / 1000f;
        float rafLatencyMs = (rafLatencyUs & 0xFFFFFFFFL) / 1000f;
        float swapBlockMs = (swapBlockUs & 0xFFFFFFFFL) / 1000f;

        // Row 1: FPS + frame time + dropped + command drops
        StringBuilder sb = new StringBuilder();
        sb.append(String.format("%.1f FPS  %.1fms", fps, frameMs));
        if (dropped > 0) {
            sb.append(String.format("  D:%d", dropped & 0xFFFFFFFFL));
        }
        if (cmdDrops > 0) {
            sb.append(String.format("  CD:%d", cmdDrops & 0xFFFFFFFFL));
        }
        rowFps.setText(sb.toString());
        rowFps.setTextColor(fps < 25 ? WARN_COLOR : TEXT_COLOR);

        // Row 2: rendering latency + damage/upload optimization counters
        long totalDmgFrames = (partialDamageFrames & 0xFFFFFFFFL) + (fullSurfaceFrames & 0xFFFFFFFFL);
        String dmgInfo = totalDmgFrames > 0
                ? String.format("  P:%d F:%d %dkpx",
                    partialDamageFrames & 0xFFFFFFFFL,
                    fullSurfaceFrames & 0xFFFFFFFFL,
                    damageAreaKpx & 0xFFFFFFFFL)
                : "";
        String uploadInfo = (uploadFrameReject > 0 || droppedUploadRecov > 0)
                ? String.format("  UR:%d DR:%d",
                    uploadFrameReject & 0xFFFFFFFFL,
                    droppedUploadRecov & 0xFFFFFFFFL)
                : "";
        rowRender.setText(String.format(
                "RAF: %.1fms  Swap: %.1fms  UQ: %d  GM: %d%s%s",
                rafLatencyMs,
                swapBlockMs,
                uploadQueueDepth & 0xFFFFFFFFL,
                glyphAtlasMiss & 0xFFFFFFFFL,
                dmgInfo,
                uploadInfo));
        rowRender.setTextColor((rafLatencyUs > 0 || swapBlockUs > 0) ? LABEL_COLOR : TEXT_COLOR);

        // Queue / cache observability row (v4).  Collector pending
        // bytes is shown in KiB to keep the line short; a rising
        // render queue depth near the 512 cap means the host
        // thread is on the verge of blocking in CommandSender::send.
        int collectorKb = (int) ((collectorPending & 0xFFFFFFFFL) / 1024);
        rowQueue.setText(String.format(
                "Q: %d  PB: %dKB  WR: %d  SK: %d  DU: %d",
                renderQueueLen & 0xFFFFFFFFL,
                collectorKb,
                webglErrOverflow & 0xFFFFFFFFL,
                skImageWrappers & 0xFFFFFFFFL,
                deferredUploads & 0xFFFFFFFFL));
        // Highlight when queue is > 75% of its 512 cap, or when
        // any WebGL error has been dropped.
        boolean queueWarn = (renderQueueLen & 0xFFFFFFFFL) > 384
                || (webglErrOverflow & 0xFFFFFFFFL) > 0;
        rowQueue.setTextColor(queueWarn ? WARN_COLOR : TEXT_COLOR);

        // Canvas2D zero-readback snapshot row (v5).
        // Snap = snapshots taken (FBO blit succeeded);
        // Up   = snapshots consumed by texImage2D (the cocos hot path);
        // FB   = JS fell back to the legacy CPU getImageData;
        // FR   = `migo._force_readback(imageData)` calls (slow CPU readback).
        // Steady state for a Cocos text-heavy game: Snap≈Up, FB=0, FR=0.
        rowSnap.setText(String.format(
                "Snap: %d  Up: %d  FB: %d  FR: %d",
                snapTaken & 0xFFFFFFFFL,
                snapUpload & 0xFFFFFFFFL,
                snapFallback & 0xFFFFFFFFL,
                snapForcedReadback & 0xFFFFFFFFL));
        // Warn when a non-trivial fraction of getImageData calls
        // are falling back; the snapshot path was supposed to handle
        // that traffic.  Threshold: any fallbacks at all if the
        // snapshot path has produced output (rules out the GLES 2
        // case where fallback is the only option).
        boolean snapWarn = (snapTaken & 0xFFFFFFFFL) > 0
                && (snapFallback & 0xFFFFFFFFL) > 0;
        rowSnap.setTextColor(snapWarn ? WARN_COLOR : TEXT_COLOR);

        // First render (one-shot)
        if (!firstRenderShown && firstFrameMs > 0) {
            firstRenderShown = true;
            firstRenderMs = firstFrameMs & 0xFFFFFFFFL;
            updateTimingRow();
        }

        refreshMemory();
        refreshCpu();

        // Fatal error row
        if (fatalError != 0) {
            if (rowFatal == null) {
                rowFatal = createRow("FATAL: " + fatalError);
                rowFatal.setTextColor(ERROR_COLOR);
            } else {
                rowFatal.setText("FATAL: " + fatalError);
            }
        }
    }

    private void updateTimingRow() {
        String startup = startupTimeMs >= 0 ? startupTimeMs + "ms" : "--";
        String firstRender = firstRenderShown ? firstRenderMs + "ms" : "--";
        rowTiming.setText(String.format("Start: %s (1st: %s)", startup, firstRender));
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
                    float cpuPercent = (deltaCpu * 10.0f * 100.0f) / deltaUptime;
                    cpuPercent = Math.min(cpuPercent, 999.9f);
                    rowCpu.setText(String.format("CPU: %.1f%%", cpuPercent));
                    rowCpu.setTextColor(cpuPercent > 80 ? WARN_COLOR : TEXT_COLOR);
                }
            }
            prevCpuTime = cpuTime;
            prevUptime = uptimeMs;
        } catch (Exception e) {
            // CPU stats not available
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
