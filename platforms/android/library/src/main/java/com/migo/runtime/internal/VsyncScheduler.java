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
