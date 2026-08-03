package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.util.concurrent.ConcurrentHashMap;
import org.junit.Test;

public final class ResourceCleanupTest {
    private static final class Resource {
        boolean active = true;
        boolean failDestroy;

        Resource(boolean failDestroy) {
            this.failDestroy = failDestroy;
        }

        void destroy() {
            if (failDestroy) throw new IllegalStateException("destroy failed");
            active = false;
        }
    }

    @Test
    public void runAllAggregatesFailuresWithoutSkippingIndependentActions() {
        int[] attempts = {0};

        try {
            ResourceCleanup.runAll(
                    () -> { attempts[0]++; throw new IllegalStateException("first"); },
                    () -> attempts[0]++,
                    () -> { attempts[0]++; throw new IllegalArgumentException("last"); });
            fail("cleanup failure was swallowed");
        } catch (IllegalStateException expected) {
            assertEquals("first", expected.getMessage());
            assertEquals(1, expected.getSuppressed().length);
            assertEquals("last", expected.getSuppressed()[0].getMessage());
        }

        assertEquals(3, attempts[0]);
    }

    @Test
    public void destroyMatchingRemovesSuccessesAndRetainsFailuresForRetry() {
        ConcurrentHashMap<Integer, Resource> resources = new ConcurrentHashMap<>();
        Resource first = new Resource(false);
        Resource failed = new Resource(true);
        Resource last = new Resource(false);
        resources.put(1, first);
        resources.put(2, failed);
        resources.put(3, last);

        try {
            ResourceCleanup.destroyMatching(resources, ignored -> true, Resource::destroy);
            fail("destroy failure was swallowed");
        } catch (IllegalStateException expected) {
            assertEquals("destroy failed", expected.getMessage());
        }

        assertFalse(first.active);
        assertFalse(last.active);
        assertTrue(failed.active);
        assertEquals(1, resources.size());
        assertSame(failed, resources.get(2));

        failed.failDestroy = false;
        ResourceCleanup.destroyMatching(resources, ignored -> true, Resource::destroy);
        assertFalse(failed.active);
        assertTrue(resources.isEmpty());
    }

    @Test
    public void releaseReturnsNullOnlyAfterConfirmedRelease() {
        Resource failed = new Resource(true);
        Resource handle = failed;
        try {
            handle = ResourceCleanup.release(handle, Resource::destroy);
            fail("release failure was swallowed");
        } catch (IllegalStateException expected) {
            assertSame(failed, handle);
            assertTrue(handle.active);
        }

        failed.failDestroy = false;
        handle = ResourceCleanup.release(handle, Resource::destroy);
        assertEquals(null, handle);
        assertFalse(failed.active);
    }

    @Test
    public void retainedReplacementPublishesOnlyAfterExistingDestroySucceeds() {
        Resource existing = new Resource(true);
        Resource replacement = new Resource(false);
        ResourceCleanup.Retained<Resource> retained = new ResourceCleanup.Retained<>(existing);

        try {
            retained.replace(replacement, Resource::destroy);
            fail("replacement ignored existing destroy failure");
        } catch (IllegalStateException expected) {
            assertSame(existing, retained.get());
        }

        existing.failDestroy = false;
        retained.replace(replacement, Resource::destroy);
        assertFalse(existing.active);
        assertSame(replacement, retained.get());
    }

    @Test
    public void retainedDestroyKeepsSpecificResourceUntilRetrySucceeds() {
        Resource resource = new Resource(true);
        ResourceCleanup.Retained<Resource> retained = new ResourceCleanup.Retained<>(resource);

        try {
            retained.destroy(Resource::destroy);
            fail("destroy failure released retained resource");
        } catch (IllegalStateException expected) {
            assertSame(resource, retained.get());
        }

        resource.failDestroy = false;
        retained.destroy(Resource::destroy);
        assertEquals(null, retained.get());
        assertFalse(resource.active);
    }
}
