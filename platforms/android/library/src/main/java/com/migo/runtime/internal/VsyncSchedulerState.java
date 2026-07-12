package com.migo.runtime.internal;

/**
 * Pure, Android-free state machine for the R1 demand-driven one-shot VSync
 * scheduler.
 *
 * <p>Holds only the four scheduling booleans and returns the {@link Action} the
 * caller must perform against the {@code Choreographer} on the UI thread. It
 * deliberately has <b>no</b> {@code android.*} dependency so it can be unit
 * tested on a host JVM (see {@code scripts/test-vsync-scheduler-state.sh}).
 *
 * <p>Model: a frame callback is posted only on demand ({@link #requestFrame()}),
 * never self-reposted from {@code doFrame}. Demand that cannot be posted right
 * now (stopped or surface not ready) is retained in {@code requested} and armed
 * exactly once when {@link #onStart()} / {@link #onSurfaceReady(boolean)} makes
 * the scheduler postable again. {@code callbackPosted} coalesces duplicate
 * requests so at most one callback is ever in flight.
 *
 * @hide
 */
final class VsyncSchedulerState {

    /** The Choreographer action the owning {@code VsyncScheduler} must perform. */
    enum Action {
        /** Post one frame callback. */
        POST,
        /** Remove the currently posted frame callback. */
        REMOVE,
        /** Deliver this frame's timestamp to native (no repost). */
        DELIVER,
        /** A stale/late doFrame that must not be delivered. */
        IGNORE,
        /** Do nothing. */
        NONE
    }

    private boolean running;
    private boolean surfaceReady;
    private boolean callbackPosted;
    private boolean requested;

    /** Matches the historical {@code VsyncScheduler} field defaults. */
    VsyncSchedulerState() {
        this(false, true);
    }

    VsyncSchedulerState(boolean running, boolean surfaceReady) {
        this.running = running;
        this.surfaceReady = surfaceReady;
    }

    /** A frame was requested (by native via {@code requestVsync}, on the UI thread). */
    Action requestFrame() {
        if (running && surfaceReady) {
            if (!callbackPosted) {
                callbackPosted = true;
                return Action.POST;
            }
            return Action.NONE; // already in flight — coalesce
        }
        // Cannot post now; retain the demand until we become postable again.
        requested = true;
        return Action.NONE;
    }

    Action onStart() {
        running = true;
        if (requested && surfaceReady && !callbackPosted) {
            requested = false;
            callbackPosted = true;
            return Action.POST;
        }
        return Action.NONE;
    }

    Action onStop() {
        running = false;
        if (callbackPosted) {
            // Remove the posted callback but keep the demand so start() re-arms it.
            requested = true;
            callbackPosted = false;
            return Action.REMOVE;
        }
        return Action.NONE;
    }

    Action onSurfaceReady(boolean ready) {
        surfaceReady = ready;
        if (ready) {
            if (requested && running && !callbackPosted) {
                requested = false;
                callbackPosted = true;
                return Action.POST;
            }
            return Action.NONE;
        }
        if (callbackPosted) {
            // Surface loss removes the posted callback without losing the demand.
            requested = true;
            callbackPosted = false;
            return Action.REMOVE;
        }
        return Action.NONE;
    }

    Action doFrame() {
        callbackPosted = false;
        if (!running || !surfaceReady) {
            // Stale doFrame that raced stop()/surface loss: do not deliver, and
            // retain the demand so it re-arms once postable again.
            requested = true;
            return Action.IGNORE;
        }
        // This frame services any pending demand. Crucially, we do NOT repost —
        // continuity is driven by the render thread / op re-requesting a frame
        // while demand remains.
        requested = false;
        return Action.DELIVER;
    }

    // ---- Test accessors (package-private) ----
    boolean isCallbackPosted() {
        return callbackPosted;
    }

    boolean isRequested() {
        return requested;
    }

    boolean isRunning() {
        return running;
    }

    boolean isSurfaceReady() {
        return surfaceReady;
    }
}
