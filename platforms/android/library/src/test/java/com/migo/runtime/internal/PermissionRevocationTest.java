package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public final class PermissionRevocationTest {
    private static class Events implements PermissionRevocation.ResourceTeardown {
        final List<String> values = new ArrayList<>();

        @Override public void destroyCamera(int sessionId) { values.add("camera"); }
        @Override public void destroyRecorder(int sessionId) { values.add("recorder"); }
        @Override public void destroyBluetooth(int sessionId) { values.add("bluetooth"); }
    }

    /**
     * The boolean a host reads back from `updatePermission`.
     *
     * Mutation testing found both of `update`'s returns replaceable by constants without
     * killing anything: the suite asserted which side effects happened and never the
     * verdict, which is the only part of this the embedder sees. A host that records a
     * standing decision and is told "true" for a refusal has a permission it believes is
     * set and the runtime does not.
     *
     * Both polarities, and the refusals must also not reach native at all.
     */
    @Test
    public void everyRefusalReportsFailureAndReachesNothing() {
        java.util.List<String> native_ = new java.util.ArrayList<>();
        java.util.function.BooleanSupplier reachedNative = () -> {
            native_.add("native");
            return true;
        };

        assertFalse(
                "a null scope is refused",
                PermissionRevocation.update(null, () -> false, reachedNative));
        assertFalse(
                "an empty scope is refused",
                PermissionRevocation.update("", () -> false, reachedNative));
        assertFalse(
                "a terminated session is refused",
                PermissionRevocation.update("scope.camera", () -> true, reachedNative));
        assertTrue("a refused update must not reach native", native_.isEmpty());

        assertTrue(
                "a live update reports the native verdict",
                PermissionRevocation.update("scope.camera", () -> false, reachedNative));
        assertEquals(java.util.Collections.singletonList("native"), native_);
    }

    @Test
    public void revocationTearsDownOnlyTheTargetedResource() {
        assertRevokesOnly("scope.camera", "camera");
        assertRevokesOnly("scope.record", "recorder");
        assertRevokesOnly("scope.bluetooth", "bluetooth");
    }

    @Test
    public void unrelatedScopesDoNotTearDownResources() {
        Events unrelated = new Events();
        PermissionRevocation.tearDown(7, "scope.userInfo", unrelated, () -> {});
        assertEquals(new ArrayList<>(), unrelated.values);
    }

    @Test
    public void teardownFailureTerminatesTheSessionAndPropagates() {
        PermissionRevocation.ResourceTeardown resources = new Events() {
            @Override public void destroyCamera(int sessionId) {
                throw new IllegalStateException("camera remained active");
            }
        };
        boolean[] terminated = {false};

        try {
            PermissionRevocation.tearDown(
                    7, "scope.camera", resources, () -> terminated[0] = true);
            fail("teardown failure was swallowed");
        } catch (IllegalStateException expected) {
            assertEquals("camera remained active", expected.getMessage());
        }
        assertTrue("persistent cleanup failure did not terminate the session", terminated[0]);
    }

    @Test
    public void lateUpdatesForTerminatedSessionsAreIgnored() {
        Events events = new Events();
        PermissionRevocation.update(
                "scope.camera", () -> true,
                () -> { events.values.add("native"); return true; });
        assertEquals(new ArrayList<>(), events.values);
    }

    @Test
    public void liveUpdatesReachNative() {
        Events events = new Events();
        PermissionRevocation.update(
                "scope.camera", () -> false,
                () -> { events.values.add("native"); return true; });
        assertEquals(Arrays.asList("native"), events.values);
    }

    @Test
    public void nativePermissionUpdateFailureIsNotReportedAsSuccess() {
        assertFalse(PermissionRevocation.update(
                "scope.camera", () -> false, () -> false));
    }

    private static void assertRevokesOnly(String scope, String resource) {
        Events events = new Events();
        PermissionRevocation.tearDown(7, scope, events, () -> {});
        assertEquals(Arrays.asList(resource), events.values);
    }
}
