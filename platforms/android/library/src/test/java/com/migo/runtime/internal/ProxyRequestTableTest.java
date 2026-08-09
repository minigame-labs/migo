package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.util.HashSet;
import java.util.Set;

/**
 * The table that keeps a proxied Activity result attached to its request.
 */
public final class ProxyRequestTableTest {

    @Test
    public void a_token_is_never_reissued_past_the_band_the_old_counter_wrapped_in() {
        ProxyRequestTable<String> table = new ProxyRequestTable<>();
        Set<Long> seen = new HashSet<>();

        // One more than the 55,000-wide band `10000 + (n % 55000)` cycled
        // through: the old counter hands out its first token again here, and
        // the request holding it loses its callback.
        for (int i = 0; i < 55_001; i++) {
            long token = table.register("request " + i);
            if (!seen.add(token)) {
                fail("token " + token + " was issued twice, at launch " + i);
            }
        }

        assertEquals(55_001, seen.size());
        assertEquals(55_001, table.size());
    }

    @Test
    public void an_entry_survives_any_number_of_later_launches() {
        // The eviction this replaced dropped entries older than sixty seconds
        // whenever a new launch arrived. There is no clock here to make old
        // mean anything, so the only way an entry can leave is by being taken.
        ProxyRequestTable<String> table = new ProxyRequestTable<>();
        long first = table.register("the picker the user is still browsing");

        for (int i = 0; i < 1_000; i++) {
            table.register("later launch " + i);
        }

        assertSame("the picker the user is still browsing", table.take(first));
    }

    @Test
    public void a_request_is_taken_exactly_once() {
        // Both the result and the proxy's destruction try to take the entry;
        // whichever loses must get nothing rather than deliver a second time.
        ProxyRequestTable<String> table = new ProxyRequestTable<>();
        long token = table.register("only");

        assertSame("only", table.take(token));
        assertNull(table.take(token));
        assertEquals(0, table.size());
    }

    @Test
    public void taking_one_request_leaves_the_others_untouched() {
        ProxyRequestTable<String> table = new ProxyRequestTable<>();
        long first = table.register("first");
        long second = table.register("second");

        assertSame("first", table.take(first));
        assertSame("second", table.take(second));
        assertEquals(0, table.size());
    }

    @Test
    public void an_unknown_token_takes_nothing() {
        ProxyRequestTable<String> table = new ProxyRequestTable<>();
        table.register("live");

        assertNull(table.take(-1L));
        assertNull(table.take(0L));
        assertNull(table.take(Long.MAX_VALUE));
        assertEquals(1, table.size());
    }

    @Test
    public void the_last_token_is_issued_once_and_then_exhaustion_is_permanent() {
        ProxyRequestTable<String> table = new ProxyRequestTable<>(Long.MAX_VALUE - 1);

        assertEquals(Long.MAX_VALUE, table.register("the last one"));

        // Asked twice on purpose: an "exhausted" that recovers on the next call
        // is a wrap wearing an error's name, and the token it wraps to may
        // still name a live entry.
        for (int attempt = 0; attempt < 2; attempt++) {
            try {
                table.register("one too many");
                fail("a token was issued past the end of the space, attempt " + attempt);
            } catch (IllegalStateException exhausted) {
                assertEquals("proxy request tokens exhausted", exhausted.getMessage());
            }
        }

        // Nothing was recorded for the refused registrations.
        assertEquals(1, table.size());
    }
}
