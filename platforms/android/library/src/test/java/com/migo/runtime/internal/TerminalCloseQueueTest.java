package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.List;
import static org.junit.Assert.assertFalse;
import org.junit.Test;

public final class TerminalCloseQueueTest {
    /**
     * A refused post is reported as a refusal, and leaves nothing pending.
     *
     * The caller adds a suppressed "failed to schedule terminal close" on false, so a queue
     * that always claimed success would swallow the one signal that a session was left
     * un-closed. Mutating the return to a constant survived: the existing test only
     * exercises a poster that accepts.
     *
     * The retry is what proves the target was released: a queue that reported false but
     * kept the target pending would refuse the second attempt as a duplicate.
     */
    @Test
    public void aRefusedPostIsReportedAndLeavesNothingPending() {
        TerminalCloseQueue<String> queue = new TerminalCloseQueue<>();
        java.util.List<String> closed = new java.util.ArrayList<>();

        assertFalse(
                "a null target is refused rather than queued",
                queue.schedule(null, runnable -> true, closed::add));
        assertFalse(
                "a poster that refuses is reported as a refusal",
                queue.schedule("session", runnable -> false, closed::add));
        assertTrue("a refused post closes nothing", closed.isEmpty());

        assertTrue(
                "the target is free to be scheduled again",
                queue.schedule("session", runnable -> {
                    runnable.run();
                    return true;
                }, closed::add));
        assertEquals(java.util.Collections.singletonList("session"), closed);
    }

    @Test
    public void capturesSessionAndDeduplicatesUntilPostedCloseFinishes() {
        TerminalCloseQueue<Object> queue = new TerminalCloseQueue<>();
        List<Runnable> posted = new ArrayList<>();
        List<Object> closed = new ArrayList<>();
        Object session = new Object();

        assertTrue(queue.schedule(session, runnable -> posted.add(runnable), closed::add));
        assertTrue(queue.schedule(session, runnable -> posted.add(runnable), closed::add));
        assertEquals(1, posted.size());

        posted.remove(0).run();
        assertEquals(1, closed.size());
        assertEquals(session, closed.get(0));

        assertTrue(queue.schedule(session, runnable -> posted.add(runnable), closed::add));
        assertEquals(1, posted.size());
    }
}
