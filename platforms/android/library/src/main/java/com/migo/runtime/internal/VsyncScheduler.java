package com.migo.runtime.internal;

import android.view.Choreographer;

/**
 * Drives the render loop using Android's Choreographer for hardware VSync
 * alignment — <b>on demand</b> (R1).
 * <p>
 * Unlike the historical always-on scheduler, each frame callback is posted only
 * when a frame is actually requested ({@link #requestFrame()}), and {@code
 * doFrame} never self-reposts. Continuity during active animation is driven by
 * the native render thread (and {@code op_await_next_frame}) re-requesting a
 * frame through {@code NativeExports.requestVsync} while demand remains; when
 * demand clears, the clock simply stops. This eliminates the ~60–120 wakeups/s
 * that a permanently reposting Choreographer burned on a static foreground
 * screen.
 * <p>
 * All methods run on the UI thread (Choreographer requires a Looper). The
 * scheduling decisions live in the Android-free {@link VsyncSchedulerState} so
 * they are unit-testable on a host JVM.
 * <p>
 * The target frame rate is not this class's concern. The native {@code
 * FrameScheduler} is its single authority: it decimates the vsyncs delivered
 * here and derives the display period from their timestamps, so nothing about
 * the display has to be reported from Java (R1 removed the Java-side {@code
 * frameSkipInterval}/{@code frameCounter} second FPS authority).
 *
 * @hide
 */
public final class VsyncScheduler implements Choreographer.FrameCallback {

    private final int sessionId;
    private final VsyncSchedulerState state = new VsyncSchedulerState();

    public VsyncScheduler(int sessionId) {
        this.sessionId = sessionId;
    }

    /**
     * Enable posting frame callbacks. Any demand retained while stopped is armed
     * now. Must be called on the UI thread.
     */
    public void start() {
        applyScheduleAction(state.onStart());
    }

    /**
     * Stop posting frame callbacks, removing any in-flight callback. Legitimate
     * pending demand is retained and re-armed on the next {@link #start()}.
     * Must be called on the UI thread.
     */
    public void stop() {
        applyScheduleAction(state.onStop());
    }

    /**
     * Mark whether a live surface is available. Losing the surface removes any
     * posted callback without losing pending demand; regaining it re-arms.
     * Must be called on the UI thread.
     */
    public void setSurfaceReady(boolean surfaceReady) {
        applyScheduleAction(state.onSurfaceReady(surfaceReady));
    }

    /**
     * Request exactly one frame callback. Called on the UI thread (via
     * {@code NativeExports.requestVsync} → {@code GameSession.requestVsyncFrame})
     * when the native render thread / RAF loop has demand. Duplicate requests
     * while a callback is already in flight are coalesced; a request that cannot
     * be posted right now (stopped / no surface) is retained and armed later.
     */
    public void requestFrame() {
        applyScheduleAction(state.requestFrame());
    }

    @Override
    public void doFrame(long frameTimeNanos) {
        VsyncSchedulerState.Action action = state.doFrame();
        if (action == VsyncSchedulerState.Action.DELIVER) {
            NativeBridge.onVsync(sessionId, frameTimeNanos);
        }
        // IGNORE (stale doFrame after stop/surface loss) and NONE: do nothing.
        // Never repost here — continuity comes from an explicit requestFrame().
    }

    private void applyScheduleAction(VsyncSchedulerState.Action action) {
        switch (action) {
            case POST:
                Choreographer.getInstance().postFrameCallback(this);
                break;
            case REMOVE:
                Choreographer.getInstance().removeFrameCallback(this);
                break;
            default:
                // NONE — the scheduling methods never return DELIVER/IGNORE.
                break;
        }
    }
}
