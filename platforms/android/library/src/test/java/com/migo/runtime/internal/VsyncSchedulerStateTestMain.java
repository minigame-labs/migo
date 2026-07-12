package com.migo.runtime.internal;

import com.migo.runtime.internal.VsyncSchedulerState.Action;

/**
 * Host-JVM test main for the R1 demand-driven one-shot VSync state machine.
 * Runs without Android — {@link VsyncSchedulerState} is deliberately free of any
 * android.* dependency. Invoked by scripts/test-vsync-scheduler-state.sh.
 */
public final class VsyncSchedulerStateTestMain {
    private static int checks = 0;

    private static void expect(Action actual, Action expected, String msg) {
        checks++;
        if (actual != expected) {
            throw new AssertionError("FAIL: " + msg + " — expected " + expected + " got " + actual);
        }
    }

    private static void expectTrue(boolean cond, String msg) {
        checks++;
        if (!cond) {
            throw new AssertionError("FAIL: " + msg);
        }
    }

    public static void main(String[] args) {
        oneRequestOneCallbackNoRepost();
        retainedAcrossStopThenArmedOnce();
        retainedWhileSurfaceNotReady();
        staleDoFrameAfterStopIgnored();
        surfaceLossReacquireNotLost();
        System.out.println("VsyncSchedulerState: ALL " + checks + " checks PASS");
    }

    /** running starts false, surfaceReady true (matches VsyncScheduler field defaults). */
    private static VsyncSchedulerState started() {
        VsyncSchedulerState s = new VsyncSchedulerState();
        s.onStart(); // running=true; no pending demand yet => NONE
        return s;
    }

    private static void oneRequestOneCallbackNoRepost() {
        VsyncSchedulerState s = started();
        expect(s.requestFrame(), Action.POST, "first request arms one callback");
        expect(s.requestFrame(), Action.NONE, "duplicate request is coalesced");
        expect(s.doFrame(), Action.DELIVER, "doFrame delivers the vsync");
        expectTrue(!s.isCallbackPosted(), "doFrame clears callbackPosted");
        // No self-repost: doFrame returned DELIVER (not POST). The next demand re-arms.
        expect(s.requestFrame(), Action.POST, "next request arms again: one request => one callback");
    }

    private static void retainedAcrossStopThenArmedOnce() {
        VsyncSchedulerState s = started();
        expect(s.requestFrame(), Action.POST, "arm");
        expect(s.onStop(), Action.REMOVE, "stop removes the posted callback");
        expectTrue(s.isRequested(), "stop retains legitimate pending demand");
        expect(s.onStart(), Action.POST, "start re-arms retained demand exactly once");
        expect(s.onStart(), Action.NONE, "second start is a no-op (no duplicate post)");
    }

    private static void retainedWhileSurfaceNotReady() {
        VsyncSchedulerState s = started();
        expect(s.onSurfaceReady(false), Action.NONE, "no posted callback to remove");
        expect(s.requestFrame(), Action.NONE, "request while surface not ready is latched, not posted");
        expectTrue(s.isRequested(), "demand retained across surface-not-ready");
        expect(s.onSurfaceReady(true), Action.POST, "surface ready arms the retained demand");
    }

    private static void staleDoFrameAfterStopIgnored() {
        VsyncSchedulerState s = started();
        expect(s.requestFrame(), Action.POST, "arm");
        expect(s.onStop(), Action.REMOVE, "stop");
        // Choreographer may still dispatch a doFrame that raced removeFrameCallback.
        expect(s.doFrame(), Action.IGNORE, "stale doFrame after stop is ignored, not delivered");
        expectTrue(s.isRequested(), "stale doFrame retains demand");
        expect(s.onStart(), Action.POST, "start re-arms after a stale doFrame");
    }

    private static void surfaceLossReacquireNotLost() {
        VsyncSchedulerState s = started();
        expect(s.requestFrame(), Action.POST, "arm");
        expect(s.onSurfaceReady(false), Action.REMOVE, "surface loss removes the posted callback");
        expectTrue(s.isRequested(), "surface loss retains demand");
        expect(s.onSurfaceReady(true), Action.POST, "surface reacquire re-arms: the request is not lost");
    }
}
