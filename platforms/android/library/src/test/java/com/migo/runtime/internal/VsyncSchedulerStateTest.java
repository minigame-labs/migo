package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import com.migo.runtime.internal.VsyncSchedulerState.Action;
import org.junit.Test;

/**
 * Host-JVM tests for the demand-driven one-shot VSync state machine.
 *
 * <p>Runs without Android — {@link VsyncSchedulerState} is deliberately free of
 * any android.* dependency, which is what makes the scheduling rules testable at
 * all.
 */
public final class VsyncSchedulerStateTest {
    /** running starts false, surfaceReady true (matches VsyncScheduler field defaults). */
    private static VsyncSchedulerState started() {
        VsyncSchedulerState state = new VsyncSchedulerState();
        state.onStart(); // running=true; no pending demand yet => NONE
        return state;
    }

    private static void expect(Action actual, Action expected, String message) {
        assertEquals(message, expected, actual);
    }

    @Test
    public void oneRequestArmsOneCallbackAndDoesNotRepost() {
        VsyncSchedulerState state = started();
        expect(state.requestFrame(), Action.POST, "first request arms one callback");
        expect(state.requestFrame(), Action.NONE, "duplicate request is coalesced");
        expect(state.doFrame(), Action.DELIVER, "doFrame delivers the vsync");
        assertFalse("doFrame clears callbackPosted", state.isCallbackPosted());
        // No self-repost: doFrame returned DELIVER (not POST). The next demand re-arms.
        expect(state.requestFrame(), Action.POST, "next request arms again: one request => one callback");
    }

    @Test
    public void demandIsRetainedAcrossStopAndRearmedExactlyOnce() {
        VsyncSchedulerState state = started();
        expect(state.requestFrame(), Action.POST, "arm");
        expect(state.onStop(), Action.REMOVE, "stop removes the posted callback");
        assertTrue("stop retains legitimate pending demand", state.isRequested());
        expect(state.onStart(), Action.POST, "start re-arms retained demand exactly once");
        expect(state.onStart(), Action.NONE, "second start is a no-op (no duplicate post)");
    }

    @Test
    public void demandIsRetainedWhileTheSurfaceIsNotReady() {
        VsyncSchedulerState state = started();
        expect(state.onSurfaceReady(false), Action.NONE, "no posted callback to remove");
        expect(state.requestFrame(), Action.NONE, "request while surface not ready is latched, not posted");
        assertTrue("demand retained across surface-not-ready", state.isRequested());
        expect(state.onSurfaceReady(true), Action.POST, "surface ready arms the retained demand");
    }

    @Test
    public void staleDoFrameAfterStopIsIgnoredWithoutLosingDemand() {
        VsyncSchedulerState state = started();
        expect(state.requestFrame(), Action.POST, "arm");
        expect(state.onStop(), Action.REMOVE, "stop");
        // Choreographer may still dispatch a doFrame that raced removeFrameCallback.
        expect(state.doFrame(), Action.IGNORE, "stale doFrame after stop is ignored, not delivered");
        assertTrue("stale doFrame retains demand", state.isRequested());
        expect(state.onStart(), Action.POST, "start re-arms after a stale doFrame");
    }

    @Test
    public void surfaceLossAndReacquireDoesNotLoseTheRequest() {
        VsyncSchedulerState state = started();
        expect(state.requestFrame(), Action.POST, "arm");
        expect(state.onSurfaceReady(false), Action.REMOVE, "surface loss removes the posted callback");
        assertTrue("surface loss retains demand", state.isRequested());
        expect(state.onSurfaceReady(true), Action.POST, "surface reacquire re-arms: the request is not lost");
    }
}
