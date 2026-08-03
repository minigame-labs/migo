package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.fail;

import java.util.ArrayList;
import java.util.List;
import org.junit.Test;

public final class SessionPermissionSinkTest {
    @Test
    public void nativeGrantFailureReportsSchedulesCloseThrowsAndLeavesDenied() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(true, gate.open(101));
        List<RuntimeException> reported = new ArrayList<>();
        int[] scheduled = {0};
        NativeExports.SessionPermissionSink sink = new NativeExports.SessionPermissionSink(
                101,
                gate,
                () -> false,
                (scope, granted, failure) -> reported.add(failure),
                () -> {
                    scheduled[0]++;
                    return true;
                });

        RuntimeException failure;
        NativeMethods.setPermissionUpdaterForTests((sessionId, scope, granted) -> false);
        try {
            failure = expectFailure(() -> sink.setScope("scope.camera", true));
        } finally {
            NativeMethods.resetPermissionUpdaterForTests();
        }

        assertEquals("native permission update failed", failure.getMessage());
        assertEquals(1, reported.size());
        assertEquals(1, scheduled[0]);
        assertNull(gate.register(101, "scope.camera"));
    }

    @Test
    public void nativeRevokeFailureStillPublishesDeniedBeforeThrowing() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(true, gate.open(102));
        assertNull(gate.update(102, "scope.camera", true, () -> true).failure());
        assertNotNull(gate.register(102, "scope.camera"));
        int[] reports = {0};
        int[] scheduled = {0};
        NativeExports.SessionPermissionSink sink = new NativeExports.SessionPermissionSink(
                102,
                gate,
                () -> false,
                (scope, granted, failure) -> reports[0]++,
                () -> {
                    scheduled[0]++;
                    return true;
                });

        NativeMethods.setPermissionUpdaterForTests((sessionId, scope, granted) -> false);
        try {
            expectFailure(() -> sink.setScope("scope.camera", false));
        } finally {
            NativeMethods.resetPermissionUpdaterForTests();
        }

        assertEquals(1, reports[0]);
        assertEquals(1, scheduled[0]);
        assertNull(gate.register(102, "scope.camera"));
    }

    @Test
    public void lateScopeUpdateAfterSessionTerminationIsIgnored() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(true, gate.open(103));
        int[] nativeUpdates = {0};
        int[] reports = {0};
        int[] scheduled = {0};
        NativeExports.SessionPermissionSink sink = new NativeExports.SessionPermissionSink(
                103,
                gate,
                () -> true,
                (scope, granted, failure) -> reports[0]++,
                () -> {
                    scheduled[0]++;
                    return true;
                });

        NativeMethods.setPermissionUpdaterForTests((sessionId, scope, granted) -> {
            nativeUpdates[0]++;
            return false;
        });
        try {
            sink.setScope("scope.camera", true);
        } finally {
            NativeMethods.resetPermissionUpdaterForTests();
        }

        assertEquals(0, nativeUpdates[0]);
        assertEquals(0, reports[0]);
        assertEquals(0, scheduled[0]);
        assertNull(gate.register(103, "scope.camera"));
    }

    private static RuntimeException expectFailure(Runnable action) {
        try {
            action.run();
            fail("permission update failure returned normally");
            return null;
        } catch (RuntimeException expected) {
            return expected;
        }
    }
}
