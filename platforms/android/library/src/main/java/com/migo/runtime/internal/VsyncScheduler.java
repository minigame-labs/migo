package com.migo.runtime.internal;

import android.view.Choreographer;

/**
 * Drives the render loop using Android's Choreographer for hardware VSync alignment.
 * <p>
 * Each frame callback sends the VSync timestamp to the native render thread via JNI.
 * The Choreographer runs on the UI thread, ensuring minimal latency between the
 * display VSync signal and the native render thread's frame processing.
 *
 * @hide
 */
public final class VsyncScheduler implements Choreographer.FrameCallback {

    private final int sessionId;
    private volatile boolean running = false;
    private volatile boolean surfaceReady = true;
    private boolean callbackPosted = false;
    private int frameSkipInterval = 1;  // 1 = no skip, 2 = skip every other frame
    private int frameCounter = 0;
    private long refreshPeriodNanos = 16_666_667L;  // default 60Hz

    public VsyncScheduler(int sessionId) {
        this.sessionId = sessionId;
    }

    /**
     * Start posting frame callbacks. Must be called on the UI thread.
     */
    public void start() {
        if (!running) {
            running = true;
            scheduleIfNeeded();
        }
    }

    /**
     * Stop posting frame callbacks. Must be called on the UI thread.
     */
    public void stop() {
        running = false;
        unschedule();
    }

    /**
     * Set the target FPS. On high-refresh displays, VSync callbacks will be
     * skipped to match the target rate. Also notifies the native render thread
     * of the display refresh period for frame budget calculations.
     * @param targetFps desired frame rate (e.g., 60)
     * @param displayRefreshRate actual display refresh rate (e.g., 120)
     */
    public void setTargetFps(int targetFps, float displayRefreshRate) {
        if (targetFps > 0 && displayRefreshRate > 0) {
            this.frameSkipInterval = Math.max(1, Math.round(displayRefreshRate / targetFps));
        } else {
            this.frameSkipInterval = 1;
        }
        if (displayRefreshRate > 0) {
            this.refreshPeriodNanos = Math.round(1_000_000_000.0 / displayRefreshRate);
        }
        NativeMethods.setDisplayRefreshRate(sessionId, this.refreshPeriodNanos);
    }

    public void setSurfaceReady(boolean surfaceReady) {
        this.surfaceReady = surfaceReady;
        if (surfaceReady) {
            scheduleIfNeeded();
        } else {
            unschedule();
        }
    }

    @Override
    public void doFrame(long frameTimeNanos) {
        callbackPosted = false;
        if (!running || !surfaceReady) return;
        if (frameSkipInterval > 1 && ++frameCounter % frameSkipInterval != 0) {
            scheduleIfNeeded();
            return;
        }
        NativeBridge.onVsync(sessionId, frameTimeNanos);
        scheduleIfNeeded();
    }

    private void scheduleIfNeeded() {
        if (!running || !surfaceReady || callbackPosted) {
            return;
        }
        callbackPosted = true;
        Choreographer.getInstance().postFrameCallback(this);
    }

    private void unschedule() {
        if (!callbackPosted) {
            return;
        }
        callbackPosted = false;
        Choreographer.getInstance().removeFrameCallback(this);
    }
}
