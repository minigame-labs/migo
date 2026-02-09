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

    public VsyncScheduler(int sessionId) {
        this.sessionId = sessionId;
    }

    /**
     * Start posting frame callbacks. Must be called on the UI thread.
     */
    public void start() {
        if (!running) {
            running = true;
            Choreographer.getInstance().postFrameCallback(this);
        }
    }

    /**
     * Stop posting frame callbacks. Must be called on the UI thread.
     */
    public void stop() {
        running = false;
        Choreographer.getInstance().removeFrameCallback(this);
    }

    @Override
    public void doFrame(long frameTimeNanos) {
        if (!running) return;
        NativeBridge.onVsync(sessionId, frameTimeNanos);
        // Re-post for next frame.
        Choreographer.getInstance().postFrameCallback(this);
    }
}
