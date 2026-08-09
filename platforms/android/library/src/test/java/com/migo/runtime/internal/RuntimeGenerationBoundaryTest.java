package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

/**
 * Which runtime a platform object belongs to, and when it stops belonging.
 *
 * <p>A runtime restart replaces the JavaScript isolate but not the Android
 * objects around it. A manager created by the retired runtime keeps its
 * listeners and keeps firing, and its events must not be delivered to the
 * runtime that replaced it. The generation is what separates them, and it is
 * captured where the object was created — never re-read at the moment it
 * reports, which would always look current.
 */
public final class RuntimeGenerationBoundaryTest {

    /** Distinct per test, because the boundary is process-wide state. */
    private static int nextSessionId = 9000;

    private static int freshSession() {
        int sessionId = nextSessionId++;
        RuntimeGenerationBoundary.registerSession(sessionId);
        return sessionId;
    }

    @Test
    public void a_token_is_current_until_the_runtime_it_names_is_retired() {
        int session = freshSession();
        RuntimeGenerationBoundary.Token first = RuntimeGenerationBoundary.acquire(session);

        assertEquals(1L, first.generation());
        assertTrue(first.isCurrent());

        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);
        assertFalse("the retired runtime's token survived the restart", first.isCurrent());

        RuntimeGenerationBoundary.completeRestart(session, 2L);
        assertFalse("the retired token came back once the new runtime was live",
                first.isCurrent());

        RuntimeGenerationBoundary.Token second = RuntimeGenerationBoundary.acquire(session);
        assertEquals(2L, second.generation());
        assertTrue(second.isCurrent());
        assertNotEquals(first.generation(), second.generation());
    }

    @Test
    public void a_retired_token_stays_retired_however_far_the_session_moves_on() {
        // Not "one behind": a manager held by a slow platform across several
        // restarts is as stale as one held across a single restart.
        int session = freshSession();
        RuntimeGenerationBoundary.Token first = RuntimeGenerationBoundary.acquire(session);

        for (long retired = 1L; retired <= 5L; retired++) {
            RuntimeGenerationBoundary.beginRestart(session, retired, retired + 1);
            RuntimeGenerationBoundary.completeRestart(session, retired + 1);
            assertFalse("stale at generation " + (retired + 1), first.isCurrent());
        }
    }

    @Test
    public void nothing_can_be_acquired_while_a_restart_is_in_flight() {
        // Fail closed: between the retired runtime going away and the
        // replacement being live there is no runtime to own a new manager, so a
        // late call must be refused rather than handed the old generation.
        int session = freshSession();
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);

        try {
            RuntimeGenerationBoundary.acquire(session);
            fail("a token was issued during a restart");
        } catch (IllegalStateException refused) {
            assertTrue(refused.getMessage(), refused.getMessage().contains("restarting"));
        }

        RuntimeGenerationBoundary.completeRestart(session, 2L);
        assertEquals(2L, RuntimeGenerationBoundary.acquire(session).generation());
    }

    @Test
    public void a_restart_that_names_the_wrong_generations_is_refused_and_changes_nothing() {
        int session = freshSession();
        RuntimeGenerationBoundary.Token live = RuntimeGenerationBoundary.acquire(session);

        long[][] rejected = {
            {2L, 3L},   // wrong retired: this session is at 1
            {1L, 1L},   // not an increase
            {1L, 3L},   // skips a generation
            {1L, 0L},   // not positive
        };
        for (long[] pair : rejected) {
            try {
                RuntimeGenerationBoundary.beginRestart(session, pair[0], pair[1]);
                fail("beginRestart(" + pair[0] + ", " + pair[1] + ") was accepted");
            } catch (IllegalArgumentException refused) {
                // expected
            }
            // The refusal must be total: a session left half-way into a restart
            // by a rejected call can never acquire anything again.
            assertTrue("refusing " + pair[1] + " retired the live generation", live.isCurrent());
            assertEquals(1L, RuntimeGenerationBoundary.acquire(session).generation());
        }
    }

    @Test
    public void a_completion_can_never_move_a_session_backwards() {
        int session = freshSession();
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);

        for (long wrong : new long[] {1L, 0L, -1L, Long.MIN_VALUE}) {
            try {
                RuntimeGenerationBoundary.completeRestart(session, wrong);
                fail("completeRestart(" + wrong + ") was accepted");
            } catch (IllegalArgumentException refused) {
                // expected
            }
        }

        // Still closed, and the restart in flight can still finish.
        try {
            RuntimeGenerationBoundary.acquire(session);
            fail("a rejected completion opened acquisition");
        } catch (IllegalStateException expected) {
            // expected
        }
        RuntimeGenerationBoundary.completeRestart(session, 2L);
        assertEquals(2L, RuntimeGenerationBoundary.acquire(session).generation());
    }

    @Test
    public void a_completion_whose_begin_never_arrived_still_publishes() {
        // `begin` is a notification, not a request, and it can be lost: the
        // engine calls it over JNI and can do nothing with a failure but log it.
        // Refusing the completion that follows would leave this mirror at the
        // retired generation for the rest of the session, so every manager built
        // afterwards would stamp a generation the engine drops at dispatch --
        // the whole session's Android event surface silently dead, with nothing
        // left that could recover it. The engine is the authority; a completion
        // is it saying which runtime is live.
        int session = freshSession();
        RuntimeGenerationBoundary.Token retired = RuntimeGenerationBoundary.acquire(session);

        RuntimeGenerationBoundary.completeRestart(session, 2L);

        assertFalse("the retired token survived a begin-less restart", retired.isCurrent());
        assertEquals(2L, RuntimeGenerationBoundary.acquire(session).generation());
    }

    @Test
    public void a_completion_that_outruns_its_begin_is_taken_as_authoritative() {
        // The same lost notification one step along: a `begin` arrived for one
        // restart and the completion names a later one. Accepting the higher
        // number keeps the mirror with the engine; refusing it would wedge the
        // session exactly as above.
        int session = freshSession();
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);

        RuntimeGenerationBoundary.completeRestart(session, 3L);

        assertEquals(3L, RuntimeGenerationBoundary.acquire(session).generation());
    }

    @Test
    public void an_unknown_session_is_refused_rather_than_created() {
        // Registration is the only thing that creates a session here. A restart
        // that silently created one would give a token to a session the runtime
        // never registered, and every later check against it would pass.
        int unknown = nextSessionId++;

        try {
            RuntimeGenerationBoundary.acquire(unknown);
            fail("acquired a token for an unregistered session");
        } catch (IllegalStateException expected) {
            // expected
        }
        try {
            RuntimeGenerationBoundary.beginRestart(unknown, 1L, 2L);
            fail("restarted an unregistered session");
        } catch (IllegalStateException expected) {
            // expected
        }
    }

    @Test
    public void registering_the_same_session_twice_is_refused() {
        int session = freshSession();
        try {
            RuntimeGenerationBoundary.registerSession(session);
            fail("a session was registered twice");
        } catch (IllegalStateException expected) {
            // expected
        }
    }

    @Test
    public void a_token_from_one_session_is_never_current_in_another() {
        // Sessions restart independently; a token is a claim about one of them.
        int first = freshSession();
        int second = freshSession();
        RuntimeGenerationBoundary.Token firstToken = RuntimeGenerationBoundary.acquire(first);

        RuntimeGenerationBoundary.beginRestart(second, 1L, 2L);
        RuntimeGenerationBoundary.completeRestart(second, 2L);

        assertTrue("another session's restart retired this token", firstToken.isCurrent());

        RuntimeGenerationBoundary.beginRestart(first, 1L, 2L);
        RuntimeGenerationBoundary.completeRestart(first, 2L);
        assertFalse(firstToken.isCurrent());
    }

    @Test
    public void an_unregistered_session_holds_no_current_tokens() {
        // Terminal destruction: whatever is still holding a token must find it
        // stale rather than current-by-default.
        int session = freshSession();
        RuntimeGenerationBoundary.Token token = RuntimeGenerationBoundary.acquire(session);
        assertTrue(token.isCurrent());

        RuntimeGenerationBoundary.unregisterSession(session);

        assertFalse("a token outlived its session as current", token.isCurrent());
    }
}
