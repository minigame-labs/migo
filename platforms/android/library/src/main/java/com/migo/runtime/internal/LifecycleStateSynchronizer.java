package com.migo.runtime.internal;

/** Serializes a session-state read with its application to a manager. */
final class LifecycleStateSynchronizer {
    interface StateReader {
        boolean isSuspended();
    }

    interface StateApplier {
        void setSuspended(boolean suspended);
    }

    static void synchronize(
            Object managerMonitor,
            StateReader reader,
            StateApplier applier) {
        synchronized (managerMonitor) {
            applier.setSuspended(reader.isSuspended());
        }
    }

    private LifecycleStateSynchronizer() {}
}
