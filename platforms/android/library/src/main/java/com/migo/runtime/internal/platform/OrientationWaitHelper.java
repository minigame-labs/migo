package com.migo.runtime.internal.platform;

import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.SurfaceHolder;

/**
 * Encapsulates the "wait for surface to match target orientation" state machine
 * shared by MigoGameActivity and MigoGameView.
 *
 * <p>After calling {@link DisplayCompat#setDeviceOrientation}, the system may
 * deliver surface callbacks before the rotation completes. This helper defers
 * game initialization until the surface dimensions match the requested
 * orientation, with a timeout fallback.
 */
public final class OrientationWaitHelper {

    private static final long TIMEOUT_MS = 500L;

    /** Called when the helper decides it is time to initialize the game. */
    public interface InitCallback {
        void onReadyToInit(SurfaceHolder holder);
    }

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final String tag;
    private SurfaceHolder pendingHolder;
    private Runnable timeoutRunnable;
    private String targetOrientation;

    public OrientationWaitHelper(String tag) {
        this.tag = tag;
    }

    /** Set the orientation the surface must match before init. Null = no waiting. */
    public void setTargetOrientation(String orientation) {
        this.targetOrientation = orientation;
    }

    public String getTargetOrientation() {
        return targetOrientation;
    }

    /**
     * Check whether the surface dimensions match the target orientation.
     * If no target is set, always returns true.
     */
    public boolean surfaceMatches(int width, int height) {
        if (targetOrientation == null) return true;
        boolean landscape = "landscape".equals(targetOrientation)
                || "landscapeReverse".equals(targetOrientation);
        return landscape ? width > height : height > width;
    }

    /**
     * Defer initialization: store the holder and start the timeout.
     * If the timeout fires before {@link #cancel()} or a matching
     * surfaceChanged, the callback is invoked with whatever holder is pending.
     */
    public void defer(SurfaceHolder holder, InitCallback callback) {
        boolean first = pendingHolder == null;
        pendingHolder = holder;
        if (first) {
            scheduleTimeout(callback);
        }
    }

    /** Cancel any pending timeout and clear the stored holder. */
    public void cancel() {
        pendingHolder = null;
        cancelTimeout();
    }

    /**
     * Consume the pending holder (if any), cancel the timeout, and return it.
     * Returns null if nothing was deferred.
     */
    public SurfaceHolder consumePending() {
        SurfaceHolder h = pendingHolder;
        pendingHolder = null;
        cancelTimeout();
        return h;
    }

    /** Reset all state (orientation target, pending holder, timeout). */
    public void reset() {
        targetOrientation = null;
        cancel();
    }

    private void scheduleTimeout(InitCallback callback) {
        cancelTimeout();
        timeoutRunnable = new Runnable() {
            @Override
            public void run() {
                SurfaceHolder h = pendingHolder;
                pendingHolder = null;
                timeoutRunnable = null;
                if (h != null) {
                    Log.w(tag, "orientation wait timed out, proceeding with current surface");
                    callback.onReadyToInit(h);
                }
            }
        };
        handler.postDelayed(timeoutRunnable, TIMEOUT_MS);
    }

    private void cancelTimeout() {
        if (timeoutRunnable != null) {
            handler.removeCallbacks(timeoutRunnable);
            timeoutRunnable = null;
        }
    }
}
