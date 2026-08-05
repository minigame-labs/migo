package com.migo.runtime.internal;

/** Serializes terminal cleanup and releases ownership only after resources are clean. */
public final class TerminalCleanupState {
    public static final class Result {
        private final boolean completed;
        private final RuntimeException failure;

        private Result(boolean completed, RuntimeException failure) {
            this.completed = completed;
            this.failure = failure;
        }

        public boolean completed() {
            return completed;
        }

        public RuntimeException failure() {
            return failure;
        }
    }

    private boolean running;
    private boolean complete;

    /** Makes one caller-driven attempt. Concurrent/reentrant callers do not start another. */
    public Result attempt(
            ResourceCleanup.Action resourceCleanup,
            ResourceCleanup.Action nativeShutdown,
            ResourceCleanup.Action... ownershipRelease) {
        synchronized (this) {
            if (complete) return new Result(true, null);
            if (running) return new Result(false, null);
            running = true;
        }

        RuntimeException failure = null;
        try {
            resourceCleanup.run();
            nativeShutdown.run();
            ResourceCleanup.runAll(ownershipRelease);
        } catch (RuntimeException error) {
            failure = error;
        } finally {
            synchronized (this) {
                running = false;
                complete = failure == null;
            }
        }
        return new Result(failure == null, failure);
    }

    public synchronized boolean isComplete() {
        return complete;
    }
}
