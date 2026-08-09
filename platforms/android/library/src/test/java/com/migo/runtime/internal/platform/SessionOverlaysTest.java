package com.migo.runtime.internal.platform;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.util.ArrayList;
import java.util.List;

import org.junit.Test;

/**
 * Who owns what is on screen, when more than one game is running.
 *
 * <p>These were process-wide slots: one toast and one loading overlay for the
 * whole app. Two sessions then took each other's overlays back — and because a
 * view belongs to its own Activity's decor, the removal did nothing while the
 * reference was dropped, leaving the overlay on screen with nothing able to
 * remove it. Nothing cleared the slots at teardown either, so a session that
 * closed with a toast up held a destroyed Activity alive.
 *
 * <p>The views live in the removers, so all of that is checkable here.
 */
public final class SessionOverlaysTest {

    /** Distinct per test: this is process-wide state, as it must be. */
    private static int nextSessionId = 4000;

    @Test
    public void showing_again_takes_back_what_was_already_in_the_slot() {
        int session = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(session, SessionOverlays.Slot.TOAST, () -> removed.add("first"));
        assertEquals("the first overlay was removed before anything replaced it",
                List.of(), removed);

        SessionOverlays.install(session, SessionOverlays.Slot.TOAST, () -> removed.add("second"));
        assertEquals(List.of("first"), removed);

        SessionOverlays.release(session, SessionOverlays.Slot.TOAST);
        assertEquals(List.of("first", "second"), removed);
    }

    @Test
    public void an_overlay_is_taken_back_exactly_once() {
        // A remover that runs twice removes a view that a later show has already
        // replaced, which is how an overlay disappears a moment after appearing.
        int session = nextSessionId++;
        int[] runs = {0};

        SessionOverlays.install(session, SessionOverlays.Slot.LOADING, () -> runs[0]++);
        SessionOverlays.release(session, SessionOverlays.Slot.LOADING);
        SessionOverlays.release(session, SessionOverlays.Slot.LOADING);
        SessionOverlays.releaseAll(session);

        assertEquals(1, runs[0]);
    }

    @Test
    public void the_slot_is_empty_while_its_remover_runs() {
        // Removing a view can call back into content, and content may show
        // another toast from inside that callback. The arrival must survive the
        // departure that provoked it.
        int session = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(session, SessionOverlays.Slot.TOAST, () -> {
            removed.add("first");
            SessionOverlays.install(
                    session, SessionOverlays.Slot.TOAST, () -> removed.add("reentrant"));
        });
        SessionOverlays.release(session, SessionOverlays.Slot.TOAST);

        assertEquals(List.of("first"), removed);
        SessionOverlays.release(session, SessionOverlays.Slot.TOAST);
        assertEquals("the overlay shown during the teardown was lost",
                List.of("first", "reentrant"), removed);
    }

    @Test
    public void one_session_never_takes_back_another_sessions_overlay() {
        int first = nextSessionId++;
        int second = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(first, SessionOverlays.Slot.LOADING, () -> removed.add("first"));
        SessionOverlays.install(second, SessionOverlays.Slot.LOADING, () -> removed.add("second"));
        assertEquals("showing in one session removed the other's overlay", List.of(), removed);

        SessionOverlays.release(second, SessionOverlays.Slot.LOADING);
        assertEquals(List.of("second"), removed);

        SessionOverlays.release(first, SessionOverlays.Slot.LOADING);
        assertEquals(List.of("second", "first"), removed);
    }

    @Test
    public void closing_a_session_takes_back_everything_it_owns_and_leaves_nothing_tracked() {
        // The leak: without this, a session that closes with a toast up leaves a
        // static reference to a view of a destroyed Activity.
        int session = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(session, SessionOverlays.Slot.TOAST, () -> removed.add("toast"));
        SessionOverlays.install(session, SessionOverlays.Slot.LOADING, () -> removed.add("loading"));

        SessionOverlays.releaseAll(session);

        assertEquals(2, removed.size());
        assertTrue(removed.contains("toast"));
        assertTrue(removed.contains("loading"));
        assertFalse("the closed session was still tracked", SessionOverlays.isTracked(session));
    }

    @Test
    public void closing_one_session_leaves_the_other_sessions_overlays_alone() {
        int closing = nextSessionId++;
        int running = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(closing, SessionOverlays.Slot.TOAST, () -> removed.add("closing"));
        SessionOverlays.install(running, SessionOverlays.Slot.TOAST, () -> removed.add("running"));

        SessionOverlays.releaseAll(closing);

        assertEquals(List.of("closing"), removed);
        assertTrue("the surviving session stopped being tracked",
                SessionOverlays.isTracked(running));
        SessionOverlays.release(running, SessionOverlays.Slot.TOAST);
        assertEquals(List.of("closing", "running"), removed);
    }

    @Test
    public void one_overlay_that_cannot_be_taken_back_does_not_strand_the_others() {
        int session = nextSessionId++;
        List<String> removed = new ArrayList<>();

        SessionOverlays.install(session, SessionOverlays.Slot.TOAST, () -> {
            removed.add("toast");
            throw new IllegalStateException("view already detached");
        });
        SessionOverlays.install(session, SessionOverlays.Slot.LOADING, () -> removed.add("loading"));

        try {
            SessionOverlays.releaseAll(session);
            fail("the failure was swallowed rather than reported");
        } catch (IllegalStateException reported) {
            assertEquals("view already detached", reported.getMessage());
        }

        assertEquals(2, removed.size());
        assertFalse("a failed teardown left the session tracked",
                SessionOverlays.isTracked(session));
    }

    @Test
    public void closing_a_session_that_owns_nothing_is_not_an_error() {
        SessionOverlays.releaseAll(nextSessionId++);
        SessionOverlays.release(nextSessionId++, SessionOverlays.Slot.TOAST);
    }
}
