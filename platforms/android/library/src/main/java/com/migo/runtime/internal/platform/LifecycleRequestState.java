package com.migo.runtime.internal.platform;

/**
 * Separates a caller's persistent resource request from its current physical
 * activation while a game session is lifecycle-suspended.
 *
 * @hide
 */
final class LifecycleRequestState<T> {
    enum Action {
        NONE,
        START,
        STOP,
        RESTART
    }

    private boolean requested;
    private boolean active;
    private boolean suspended;
    private boolean destroyed;
    private T request;

    LifecycleRequestState(boolean suspended) {
        this.suspended = suspended;
    }

    synchronized Action requestStart(T newRequest) {
        if (destroyed) return Action.NONE;

        request = newRequest;
        requested = true;
        if (suspended) return Action.NONE;
        if (active) return Action.RESTART;

        active = true;
        return Action.START;
    }

    synchronized Action requestStop() {
        if (destroyed) return Action.NONE;

        requested = false;
        request = null;
        if (!active) return Action.NONE;

        active = false;
        return Action.STOP;
    }

    synchronized Action suspend() {
        if (destroyed || suspended) return Action.NONE;

        suspended = true;
        if (!active) return Action.NONE;

        active = false;
        return Action.STOP;
    }

    synchronized Action resume() {
        if (destroyed || !suspended) return Action.NONE;

        suspended = false;
        if (!requested || active) return Action.NONE;

        active = true;
        return Action.START;
    }

    synchronized Action destroy() {
        if (destroyed) return Action.NONE;

        destroyed = true;
        suspended = true;
        requested = false;
        request = null;
        if (!active) return Action.NONE;

        active = false;
        return Action.STOP;
    }

    synchronized void startFailed(boolean keepRequest) {
        active = false;
        if (!keepRequest) {
            requested = false;
            request = null;
        }
    }

    synchronized T getRequest() {
        return request;
    }

    synchronized boolean isActive() {
        return active;
    }

    synchronized boolean isRequested() {
        return requested;
    }

    synchronized boolean isSuspended() {
        return suspended;
    }

    synchronized boolean isDestroyed() {
        return destroyed;
    }
}
