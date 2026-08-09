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

    /**
     * What a caller stamps when it captured no generation, and never a valid one.
     *
     * <p>For events produced by the export itself rather than by a long-lived
     * listener: a synchronous failure reply to a call the live runtime has just
     * made cannot be stale, so it must never be dropped. The engine reads any
     * non-positive value as "this producer is unfenced" and delivers it.
     */
    public static final long UNFENCED = 0L;

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
            // `next` is not recorded: the completion carries it and is
            // authoritative, so a remembered candidate would be a second opinion
            // whose only power is to refuse the first.
            state.phase = Phase.RESTARTING;
        }
    }

    /**
     * Publish {@code next} as the live generation.
     *
     * <p>Accepted whenever it moves the session forward, including when no
     * {@link #beginRestart} named it. That asymmetry with {@code beginRestart},
     * which validates its arguments and refuses, is deliberate and is about
     * which refusal a session can recover from.
     *
     * <p>These are notifications the engine makes over JNI, and it can do
     * nothing with a failure but log it. A refused <em>begin</em> costs a
     * window: acquisition is not closed while the isolate is being replaced, and
     * anything issued in it is stale the moment the completion lands — swept by
     * {@link #liveEntry} the first time the new runtime asks for it. A refused
     * <em>completion</em> is permanent: this mirror would sit at the retired
     * generation for the rest of the session, every manager built afterwards
     * would stamp a value the engine drops at dispatch, and the whole session's
     * Android event surface would go silently dead with nothing left to recover
     * it. So the completion is authoritative — the engine is the only thing that
     * knows which runtime is live — and the one rule kept is that a generation
     * never moves backwards.
     */
    public static void completeRestart(int sessionId, long next) {
        SessionState state = require(sessionId);
        synchronized (state) {
            if (next <= state.current) {
                throw new IllegalArgumentException("session " + sessionId + " is at "
                        + state.current + " and cannot complete a restart to " + next);
            }
            state.current = next;
            state.phase = Phase.ACTIVE;
        }
    }

    /**
     * The cached object for {@code sessionId}, or {@code null} once the runtime
     * that built it has been replaced.
     *
     * <p>Managers are cached per session, not per runtime, so without this the
     * isolate that replaces a retired one is handed the same object — and that
     * object stamps the generation it captured, which the engine drops at
     * dispatch. The fence would then turn "events reach the wrong runtime" into
     * "events reach nothing at all": the same feature broken in a way that looks
     * like it was never wired up. Returning {@code null} makes the caller build
     * one against the runtime that is actually running.
     *
     * <p>A retired entry is removed <em>before</em> it is destroyed, which is
     * the opposite of {@link ResourceCleanup#destroyMatching}. That one keeps an
     * entry whose destroy failed so the cleanup can be retried; here keeping it
     * is precisely wrong, because the next caller would be handed it again.
     * Removing first also means a teardown that re-enters this cache — stopping
     * a manager notifies listeners, and a listener may ask for one — is not
     * undone by a removal that comes after it.
     *
     * <p>The removal is conditional on identity, and a lost race is retried
     * rather than reported as an absence. Two callers can read the same retired
     * entry; the first sweeps it and its caller installs a replacement, and an
     * unconditional removal by the second would delete that replacement and
     * leave two managers where the cache admits one — the live object orphaned
     * on screen and a duplicate built behind it. Looking again returns the
     * replacement instead.
     *
     * @hide
     */
    public static <T extends RuntimeScoped> T liveEntry(
            ConcurrentHashMap<Integer, T> cache,
            int sessionId,
            ResourceCleanup.DestroyAction<T> destroy) {
        for (;;) {
            T existing = cache.get(sessionId);
            if (existing == null) return null;
            if (existing.runtimeToken().isCurrent()) return existing;
            if (cache.remove(sessionId, existing)) {
                // Only the thread that took it out destroys it, so a teardown
                // never runs twice on one object.
                destroy.destroy(existing);
                return null;
            }
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
