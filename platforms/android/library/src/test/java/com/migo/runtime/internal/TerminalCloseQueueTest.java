package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.List;
import org.junit.Test;

public final class TerminalCloseQueueTest {
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
