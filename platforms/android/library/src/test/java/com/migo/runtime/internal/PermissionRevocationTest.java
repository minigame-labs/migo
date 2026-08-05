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
