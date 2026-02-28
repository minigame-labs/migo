package com.migo.runtime;

import android.content.Context;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.util.TypedValue;
import android.view.Gravity;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.migo.runtime.internal.NativeBridge;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Semi-transparent debug overlay that displays real-time engine statistics.
 * <p>
 * Designed for extensibility: use {@link #addPanel(String, String)} to add custom
 * metrics panels (e.g., memory usage, draw calls, network latency).
 *
 * <h3>Usage</h3>
 * <pre>{@code
 * DebugOverlayView overlay = new DebugOverlayView(context, sessionId);
 * rootLayout.addView(overlay, new FrameLayout.LayoutParams(
 *     FrameLayout.LayoutParams.WRAP_CONTENT,
 *     FrameLayout.LayoutParams.WRAP_CONTENT,
 *     Gravity.TOP | Gravity.END));
 * overlay.startMonitoring();
 * }</pre>
 *
 * @since 1.0.0
 */
public class DebugOverlayView extends FrameLayout {

    private static final int BG_COLOR = 0xCC1A1A2E;   // dark semi-transparent
    private static final int TEXT_COLOR = 0xFFE0E0E0;  // light gray
    private static final int WARN_COLOR = 0xFFFF9800;  // orange for warnings
    private static final float TEXT_SIZE_SP = 11f;
    private static final int CORNER_RADIUS_DP = 6;
    private static final int PADDING_DP = 8;
    private static final int DEFAULT_UPDATE_INTERVAL_MS = 500;

    private final LinearLayout container;
    private final Map<String, TextView> panels = new LinkedHashMap<>();
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final int sessionId;

    private Runnable updateRunnable;
    private int updateIntervalMs = DEFAULT_UPDATE_INTERVAL_MS;

    public DebugOverlayView(Context context, int sessionId) {
        super(context);
        this.sessionId = sessionId;

        container = new LinearLayout(context);
        container.setOrientation(LinearLayout.VERTICAL);

        int padPx = dpToPx(PADDING_DP);
        container.setPadding(padPx, padPx, padPx, padPx);

        // Rounded dark background
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(BG_COLOR);
        bg.setCornerRadius(dpToPx(CORNER_RADIUS_DP));
        container.setBackground(bg);

        // Default panels
        addPanel("fps", "-- FPS");
        addPanel("frame", "-- ms");

        addView(container);
    }

    /**
     * Add a named panel row. Extensible for memory, draw calls, etc.
     *
     * @param key         Unique key for later updates via {@link #updatePanel}
     * @param initialText Initial display text
     * @return The created TextView for further customization
     */
    public TextView addPanel(String key, String initialText) {
        TextView tv = new TextView(getContext());
        tv.setTypeface(Typeface.MONOSPACE);
        tv.setTextColor(TEXT_COLOR);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SIZE_SP);
        tv.setText(initialText);
        panels.put(key, tv);
        container.addView(tv);
        return tv;
    }

    /**
     * Update a specific panel's text.
     *
     * @param key  Panel key (as passed to {@link #addPanel})
     * @param text New display text
     */
    public void updatePanel(String key, String text) {
        TextView tv = panels.get(key);
        if (tv != null) tv.setText(text);
    }

    /**
     * Set the polling interval for stats refresh.
     *
     * @param intervalMs Interval in milliseconds (default 500)
     */
    public void setUpdateInterval(int intervalMs) {
        this.updateIntervalMs = Math.max(100, intervalMs);
    }

    /**
     * Start periodic stats polling from native.
     */
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

    /**
     * Stop periodic stats polling.
     */
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

        // Treat as unsigned
        float fps = (fpsX10 & 0xFFFFFFFFL) / 10f;
        float frameMs = (frameTimeUs & 0xFFFFFFFFL) / 1000f;

        updatePanel("fps", String.format("%.1f FPS", fps));
        updatePanel("frame", String.format("%.1f ms", frameMs));

        // Show dropped frames panel only when > 0
        if (dropped > 0) {
            if (!panels.containsKey("dropped")) {
                TextView tv = addPanel("dropped", "");
                tv.setTextColor(WARN_COLOR);
            }
            updatePanel("dropped", String.format("Dropped: %d", dropped & 0xFFFFFFFFL));
        }

        if (fatalError != 0) {
            if (!panels.containsKey("fatal")) {
                TextView tv = addPanel("fatal", "");
                tv.setTextColor(WARN_COLOR);
            }
            updatePanel("fatal", "Fatal: " + fatalError);
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
