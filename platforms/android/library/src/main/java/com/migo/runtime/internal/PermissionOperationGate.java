package com.migo.runtime.internal;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.function.BooleanSupplier;

/** Linearizes permission updates, deferred framework entry, and terminal close per session. */
public final class PermissionOperationGate {
    public static final class Result {
        private final RuntimeException failure;

        private Result(RuntimeException failure) {
            this.failure = failure;
        }

        public RuntimeException failure() {
            return failure;
        }
    }

    public static final class Pending {
        private final Session session;
        private final Entry entry;
        private ResourceCleanup.Action cancellation;
        private boolean active = true;

        private Pending(
                Session session,
                Entry entry,
                ResourceCleanup.Action cancellation) {
            this.session = session;
            this.entry = entry;
            this.cancellation = cancellation;
        }

        public void setCancellation(ResourceCleanup.Action cancellation) {
            Objects.requireNonNull(cancellation, "cancellation");
            synchronized (session) {
                this.cancellation = cancellation;
            }
        }
    }

    private enum Lifecycle {
        ACTIVE,
        CLOSING
    }

    private static final class Session {
        final Object transition = new Object();
        Lifecycle lifecycle = Lifecycle.ACTIVE;
        final Map<String, Entry> scopes = new HashMap<>();
    }

    private static final class Entry {
        boolean granted;
        int activeRuns;
        final Set<Pending> pending = new HashSet<>();
    }

    private final Map<Integer, Session> sessions = new HashMap<>();
    private int highestOpenedSessionId = -1;

    /** Opens a previously unseen session. A closing tombstone is never reopened. */
    public boolean open(int sessionId) {
        synchronized (sessions) {
            if (sessionId <= highestOpenedSessionId || sessions.containsKey(sessionId)) {
                return false;
            }
            highestOpenedSessionId = sessionId;
            sessions.put(sessionId, new Session());
            return true;
        }
    }

    public Result update(
            int sessionId,
            String scope,
            boolean granted,
            BooleanSupplier updateNative) {
        Session session = session(sessionId);
        if (session == null) return rejected("permission session is not open");
        synchronized (session.transition) {
            Entry entry;
            synchronized (session) {
                if (session.lifecycle != Lifecycle.ACTIVE) {
                    return rejected("permission session is closing");
                }
                entry = entry(session, scope);
                entry.granted = false;
                if (!granted) awaitIdle(session, entry);
            }
            try {
                if (granted) {
                    requireNativeSuccess(updateNative);
                    synchronized (session) {
                        entry.granted = true;
                    }
                } else {
                    ResourceCleanup.runAll(
                            () -> {
                                synchronized (session) {
                                    cancelAll(entry);
                                }
                            },
                            () -> requireNativeSuccess(updateNative));
                }
                return new Result(null);
            } catch (RuntimeException error) {
                return new Result(error);
            }
        }
    }

    public Result revoke(int sessionId, String scope) {
        Session session = session(sessionId);
        if (session == null) return new Result(null);
        synchronized (session.transition) {
            synchronized (session) {
                Entry entry = session.scopes.get(scope);
                if (entry == null) return new Result(null);
                entry.granted = false;
                awaitIdle(session, entry);
                try {
                    cancelAll(entry);
                    return new Result(null);
                } catch (RuntimeException error) {
                    return new Result(error);
                }
            }
        }
    }

    /** Marks the session closing and retries every retained pending cancellation. */
    public Result close(int sessionId) {
        Session session = session(sessionId);
        if (session == null) return new Result(null);
        synchronized (session.transition) {
            Result result;
            synchronized (session) {
                session.lifecycle = Lifecycle.CLOSING;
                ArrayList<ResourceCleanup.Action> cancellations = new ArrayList<>();
                for (Entry entry : session.scopes.values()) {
                    entry.granted = false;
                }
                for (Entry entry : session.scopes.values()) {
                    awaitIdle(session, entry);
                    cancellations.add(() -> cancelAll(entry));
                }
                try {
                    ResourceCleanup.runAll(
                            cancellations.toArray(new ResourceCleanup.Action[0]));
                    result = new Result(null);
                } catch (RuntimeException error) {
                    result = new Result(error);
                }
            }
            if (result.failure() == null) {
                synchronized (sessions) {
                    sessions.remove(sessionId, session);
                }
            }
            return result;
        }
    }

    public Pending register(
            int sessionId,
            String scope,
            ResourceCleanup.Action cancellation) {
        Session session = session(sessionId);
        if (session == null) return null;
        synchronized (session) {
            if (session.lifecycle != Lifecycle.ACTIVE) return null;
            Entry entry = session.scopes.get(scope);
            if (entry == null || !entry.granted) return null;
            Pending pending = new Pending(session, entry, cancellation);
            entry.pending.add(pending);
            return pending;
        }
    }

    public Pending register(int sessionId, String scope) {
        return register(sessionId, scope, () -> {});
    }

    public boolean enter(Pending pending, Runnable frameworkCall) {
        if (pending == null) return false;
        synchronized (pending.session) {
            if (pending.session.lifecycle != Lifecycle.ACTIVE
                    || !pending.active
                    || !pending.entry.granted) {
                return false;
            }
            frameworkCall.run();
            return true;
        }
    }

    /** Runs an admitted scope callback while denial retains a draining lease. */
    public boolean runIfGranted(
            int sessionId,
            String scope,
            BooleanSupplier callback) {
        Objects.requireNonNull(callback, "callback");
        Session session = session(sessionId);
        if (session == null) return false;
        Entry entry;
        synchronized (session) {
            if (session.lifecycle != Lifecycle.ACTIVE) return false;
            entry = session.scopes.get(scope);
            if (entry == null || !entry.granted) return false;
            entry.activeRuns++;
        }
        try {
            return callback.getAsBoolean();
        } finally {
            synchronized (session) {
                entry.activeRuns--;
                if (entry.activeRuns == 0) session.notifyAll();
            }
        }
    }

    public void finish(Pending pending) {
        if (pending == null) return;
        synchronized (pending.session) {
            pending.active = false;
            pending.entry.pending.remove(pending);
        }
    }

    int retainedSessionCountForTests() {
        synchronized (sessions) {
            return sessions.size();
        }
    }

    private Session session(int sessionId) {
        synchronized (sessions) {
            return sessions.get(sessionId);
        }
    }

    private static Entry entry(Session session, String scope) {
        Entry existing = session.scopes.get(scope);
        if (existing != null) return existing;
        Entry created = new Entry();
        session.scopes.put(scope, created);
        return created;
    }

    private static Result rejected(String message) {
        return new Result(new IllegalStateException(message));
    }

    private static void cancelAll(Entry entry) {
        ArrayList<ResourceCleanup.Action> cancellations = new ArrayList<>();
        for (Pending pending : entry.pending) {
            cancellations.add(() -> {
                pending.cancellation.run();
                pending.active = false;
                entry.pending.remove(pending);
            });
        }
        ResourceCleanup.runAll(cancellations.toArray(new ResourceCleanup.Action[0]));
    }

    private static void awaitIdle(Session session, Entry entry) {
        boolean interrupted = false;
        while (entry.activeRuns != 0) {
            try {
                session.wait();
            } catch (InterruptedException error) {
                interrupted = true;
            }
        }
        if (interrupted) Thread.currentThread().interrupt();
    }

    private static void requireNativeSuccess(BooleanSupplier updateNative) {
        if (!updateNative.getAsBoolean()) {
            throw new IllegalStateException("native permission update failed");
        }
    }
}
