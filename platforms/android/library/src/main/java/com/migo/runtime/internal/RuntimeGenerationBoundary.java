package com.migo.runtime.internal;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Which runtime generation a platform object belongs to.
 *
 * <p>A runtime restart replaces the JavaScript isolate but not the Android
 * objects around it: a manager's listeners stay registered, a proxy Activity
 * stays queued, a dialog stays on screen. They keep producing events, and an
 * event produced for the runtime that has just been retired must not be
 * delivered to the one that replaced it.
 *
 * <p>The rule that makes that work is that a generation is <b>captured where the
 * object was created</b> and carried on every event it later reports. Re-reading
 * the current generation at report time would always match, and a comparison
 * that always matches is not a check.
 *
 * <p>The engine owns the numbering — this mirrors it, driven by the restart
 * notifications the Host sends as it replaces its isolate. A session is
 * {@code ACTIVE} at some generation, or {@code RESTARTING} between the retired
 * runtime going away and the replacement being live. Nothing can be acquired
 * while restarting: there is no runtime to own it, so the safe answer is to
 * refuse rather than hand out the generation that is going away.
 *
 * @hide
 */
public final class RuntimeGenerationBoundary {

    /** The first generation of every session, matching the engine's. */
    private static final long FIRST_GENERATION = 1L;

    private static final ConcurrentHashMap<Integer, SessionState> sSessions =
            new ConcurrentHashMap<>();

    private RuntimeGenerationBoundary() {}

    /**
     * A claim that some platform object belongs to one runtime of one session.
     *
     * <p>Immutable on purpose: a token that could be refreshed is a token whose
     * holder can be talked into believing it is current.
     */
    public static final class Token {
        private final int sessionId;
        private final long generation;

        private Token(int sessionId, long generation) {
            this.sessionId = sessionId;
            this.generation = generation;
        }

        /** The generation to stamp on every event this object reports. */
        public long generation() {
            return generation;
        }

        /**
         * Whether the runtime this token names is still the live one.
         *
         * <p>False for a session that no longer exists: an object outliving its
         * session must find itself stale, not current by default.
         */
        public boolean isCurrent() {
            SessionState state = sSessions.get(sessionId);
            if (state == null) return false;
            synchronized (state) {
                return state.phase == Phase.ACTIVE && state.current == generation;
            }
        }
    }

    private enum Phase {
        ACTIVE,
        RESTARTING,
    }

    private static final class SessionState {
        /** The live generation while {@link Phase#ACTIVE}; the retired one while restarting. */
        long current = FIRST_GENERATION;
        Phase phase = Phase.ACTIVE;
        /** The generation a restart in flight will publish; meaningless while ACTIVE. */
        long candidate;
    }

    /** Begin tracking {@code sessionId} at the first generation. */
    public static void registerSession(int sessionId) {
        if (sSessions.putIfAbsent(sessionId, new SessionState()) != null) {
            // Two runtimes numbering one session independently is the defect
            // this type exists to prevent; silently reusing the existing state
            // would hide it.
            throw new IllegalStateException("session " + sessionId + " is already registered");
        }
    }

    /** Stop tracking {@code sessionId}; every token it issued becomes stale. */
    public static void unregisterSession(int sessionId) {
        sSessions.remove(sessionId);
    }

    /**
     * A token for the live runtime of {@code sessionId}.
     *
     * @throws IllegalStateException if the session is unknown or a restart is in
     *     flight — in both cases there is no runtime for the caller to belong to.
     */
    public static Token acquire(int sessionId) {
        SessionState state = require(sessionId);
        synchronized (state) {
            if (state.phase != Phase.ACTIVE) {
                throw new IllegalStateException("session " + sessionId + " is restarting");
            }
            return new Token(sessionId, state.current);
        }
    }

    /**
     * Close acquisition for {@code retired} and expect {@code next}.
     *
     * <p>Validated against the session's own state rather than trusted: a caller
     * that has the generations wrong is a caller whose view of this session has
     * diverged, and advancing anyway would retire tokens that are still live.
     * A rejected call changes nothing at all.
     */
    public static void beginRestart(int sessionId, long retired, long next) {
        SessionState state = require(sessionId);
        synchronized (state) {
            if (next <= 0 || next != retired + 1) {
                throw new IllegalArgumentException(
                        "generation " + next + " does not succeed " + retired);
            }
            if (state.phase != Phase.ACTIVE || state.current != retired) {
                throw new IllegalArgumentException("session " + sessionId + " is not active at "
                        + retired + " (is " + state.phase + " at " + state.current + ")");
            }
            state.phase = Phase.RESTARTING;
            state.candidate = next;
        }
    }

    /**
     * Publish {@code next} as the live generation.
     *
     * <p>Only the candidate a matching {@link #beginRestart} named is accepted;
     * anything else means the caller is describing a different restart than the
     * one in flight, and the session stays closed rather than opening at a
     * generation nobody agreed on.
     */
    public static void completeRestart(int sessionId, long next) {
        SessionState state = require(sessionId);
        synchronized (state) {
            if (state.phase != Phase.RESTARTING || state.candidate != next) {
                throw new IllegalArgumentException("session " + sessionId + " is not restarting to "
                        + next + " (is " + state.phase + " at " + state.current + ")");
            }
            state.current = next;
            state.phase = Phase.ACTIVE;
        }
    }

    private static SessionState require(int sessionId) {
        SessionState state = sSessions.get(sessionId);
        if (state == null) {
            // Never created on demand: a session invented here would hand out
            // tokens the engine never numbered, and every check against them
            // would pass.
            throw new IllegalStateException("session " + sessionId + " is not registered");
        }
        return state;
    }
}
