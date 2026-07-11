package com.migo.runtime.internal.platform;

/** Terminal lifetime guard for resources referenced by queued commands. */
final class ResourceLifetime {
    private boolean released;

    synchronized boolean canRun() {
        return !released;
    }

    synchronized boolean release() {
        if (released) return false;
        released = true;
        return true;
    }
}
