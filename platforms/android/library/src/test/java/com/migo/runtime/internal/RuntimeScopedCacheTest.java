package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotSame;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

import org.junit.Test;

/**
 * What happens to a cached platform object when the runtime that created it is
 * replaced.
 *
 * <p>Fencing alone is not enough. A manager is cached per session, not per
 * runtime, so the isolate that replaces a retired one is handed the same object
 * — and that object stamps the generation it captured, which the engine now
 * drops at dispatch. Left there, the fence turns "events reach the wrong
 * runtime" into "events reach nothing at all", which is the worse of the two
 * because it looks like the feature is simply broken.
 *
 * <p>So a retired entry is not reused: it is removed, destroyed, and rebuilt
 * against the runtime that is actually running.
 */
public final class RuntimeScopedCacheTest {

    /** Distinct per test, because the boundary is process-wide state. */
    private static int nextSessionId = 7000;

    private static int freshSession() {
        int sessionId = nextSessionId++;
        RuntimeGenerationBoundary.registerSession(sessionId);
        return sessionId;
    }

    /** The smallest thing that can be runtime-scoped: a token and nothing else. */
    private static final class Fake implements RuntimeScoped {
        final RuntimeGenerationBoundary.Token token;
        boolean destroyed;

        Fake(int sessionId) {
            this.token = RuntimeGenerationBoundary.acquire(sessionId);
        }

        @Override
        public RuntimeGenerationBoundary.Token runtimeToken() {
            return token;
        }
    }

    @Test
    public void an_entry_built_by_the_live_runtime_is_returned_untouched() {
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        Fake entry = new Fake(session);
        cache.put(session, entry);

        Fake found = RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> resource.destroyed = true);

        assertSame(entry, found);
        assertTrue("the live entry stayed in the cache", cache.containsValue(entry));
        assertFalse("the live entry was destroyed", entry.destroyed);
    }

    @Test
    public void an_entry_from_a_retired_runtime_is_destroyed_removed_and_not_returned() {
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        Fake retired = new Fake(session);
        cache.put(session, retired);

        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);
        RuntimeGenerationBoundary.completeRestart(session, 2L);

        Fake found = RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> resource.destroyed = true);

        assertNull("the replacement runtime was handed the retired object", found);
        assertTrue("the retired object was left registered and firing", retired.destroyed);
        assertTrue("the retired entry survived in the cache", cache.isEmpty());

        // And what the caller builds next belongs to the runtime that is running.
        Fake rebuilt = new Fake(session);
        assertNotSame(retired, rebuilt);
        assertEquals(2L, rebuilt.token.generation());
    }

    @Test
    public void an_entry_is_retired_the_moment_a_restart_begins() {
        // Not only once the replacement is live: between the two there is no
        // runtime that owns the object, and returning it would hand the next
        // caller something that belongs to nothing.
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        cache.put(session, new Fake(session));

        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);

        assertNull(RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> resource.destroyed = true));
        assertTrue(cache.isEmpty());
    }

    @Test
    public void an_entry_whose_session_is_gone_is_retired_too() {
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        Fake orphan = new Fake(session);
        cache.put(session, orphan);

        RuntimeGenerationBoundary.unregisterSession(session);

        assertNull(RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> resource.destroyed = true));
        assertTrue("an object outliving its session kept being handed out", orphan.destroyed);
    }

    @Test
    public void a_cache_with_no_entry_destroys_nothing() {
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        AtomicInteger destroys = new AtomicInteger();

        assertNull(RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> destroys.incrementAndGet()));
        assertEquals(0, destroys.get());
    }

    @Test
    public void the_entry_leaves_the_cache_before_it_is_destroyed() {
        // Ordering, not tidiness. A destroy that hides a dialog or stops a
        // sensor can take a turn of the main looper; a caller arriving in that
        // window must not be handed the object that is being torn down.
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        cache.put(session, new Fake(session));
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);

        boolean[] visibleDuringDestroy = {true};
        RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> visibleDuringDestroy[0] = cache.containsKey(session));

        assertFalse("the doomed entry was still reachable while it was destroyed",
                visibleDuringDestroy[0]);
    }

    @Test
    public void a_replacement_installed_while_the_retired_one_is_destroyed_survives() {
        // The destroy of a retired object can re-enter this cache: tearing a
        // manager down notifies listeners, and a listener may ask for one.
        // Because the entry leaves the cache before the teardown runs, there is
        // no later removal to undo the arrival.
        int session = freshSession();
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<>();
        cache.put(session, new Fake(session));
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);
        RuntimeGenerationBoundary.completeRestart(session, 2L);

        Fake[] replacement = new Fake[1];
        RuntimeGenerationBoundary.liveEntry(cache, session, resource -> {
            replacement[0] = new Fake(session);
            cache.put(session, replacement[0]);
        });

        assertSame("the replacement was removed by the retired entry's teardown",
                replacement[0], cache.get(session));
        assertSame(replacement[0], RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> {}));
    }

    @Test
    public void losing_the_removal_race_returns_the_replacement_rather_than_destroying_it() {
        // Two callers read the same retired entry. The first sweeps it and its
        // caller installs a fresh manager; the second must not delete that
        // arrival, which would leave the live object orphaned on screen while a
        // duplicate is built behind it. The interleaving is forced rather than
        // raced so the assertion means the same thing on every run.
        int session = freshSession();
        Fake[] winner = new Fake[1];
        ConcurrentHashMap<Integer, Fake> cache = new ConcurrentHashMap<Integer, Fake>() {
            @Override
            public Fake get(Object key) {
                Fake found = super.get(key);
                if (found != null && winner[0] == null) {
                    // Between this read and the removal below, the other caller
                    // finishes its own sweep and installs the live manager.
                    winner[0] = new Fake(session);
                    super.put(session, winner[0]);
                }
                return found;
            }
        };
        Fake retired = new Fake(session);
        cache.put(session, retired);
        RuntimeGenerationBoundary.beginRestart(session, 1L, 2L);
        RuntimeGenerationBoundary.completeRestart(session, 2L);

        AtomicInteger destroys = new AtomicInteger();
        Fake found = RuntimeGenerationBoundary.liveEntry(
                cache, session, resource -> destroys.incrementAndGet());

        assertSame("the replacement was destroyed or dropped", winner[0], found);
        assertSame(winner[0], cache.get(session));
        assertEquals("the entry this caller never removed was destroyed anyway",
                0, destroys.get());
    }
}
