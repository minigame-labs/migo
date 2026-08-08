package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.fail;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import com.migo.runtime.internal.PermissionOperationGate.Admission;
import org.junit.Test;

public final class SessionPermissionSinkTest {
    /**
     * A grant that succeeds settles quietly, which is what makes the failure path a path.
     *
     * Both lambdas the sink hands the gate could be replaced by "the update failed" without
     * failing anything: every existing case drives a failing native updater, so a sink that
     * reported failure unconditionally looked correct. And the schedule check could be
     * negated, because nothing asserted the absence of a suppressed exception when the
     * scheduler accepts.
     */
    /**
     * Whether the terminal close was scheduled is carried on the thrown failure, and both
     * outcomes are asserted.
     *
     * Negating the schedule check survived every existing case: with the scheduler
     * accepting, nothing looked at the suppressed list, so "attach a bogus failure whenever
     * scheduling worked" was indistinguishable from correct. The pair below is what makes
     * the signal readable -- one suppressed exception exactly when the close could not be
     * posted, and none when it could.
     */
    @Test
    public void aRefusedTerminalCloseIsCarriedOnTheFailureAndAnAcceptedOneIsNot() {
        for (boolean posted : new boolean[] {true, false}) {
            int sessionId = posted ? 161 : 162;
            PermissionOperationGate gate = new PermissionOperationGate();
            assertEquals(Admission.ADMITTED, gate.admit(sessionId));
            NativeExports.SessionPermissionSink sink = new NativeExports.SessionPermissionSink(
                    sessionId,
                    gate,
                    () -> false,
                    (scope, granted, failure) -> { },
                    () -> posted);

            RuntimeException failure;
            NativeMethods.setPermissionUpdaterForTests((id, scope, granted) -> false);
            try {
                failure = expectFailure(() -> sink.setScope("scope.camera", true));
            } finally {
                NativeMethods.resetPermissionUpdaterForTests();
            }

            if (posted) {
                assertEquals(
                        "a scheduled close leaves nothing to suppress",
                        0,
                        failure.getSuppressed().length);
            } else {
                assertEquals(
                        "a close that could not be posted must be reported",
                        1,
                        failure.getSuppressed().length);
                assertEquals(
                        "failed to schedule terminal close",
                        failure.getSuppressed()[0].getMessage());
            }
        }
    }

    @Test
    public void aSuccessfulGrantReportsNothingAndSchedulesNothing() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(Admission.ADMITTED, gate.admit(151));
        List<RuntimeException> reported = new ArrayList<>();
        int[] scheduled = {0};
        List<String> native_ = new ArrayList<>();
        NativeExports.SessionPermissionSink sink = new NativeExports.SessionPermissionSink(
                151,
                gate,
                () -> false,
                (scope, granted, failure) -> reported.add(failure),
                () -> {
                    scheduled[0]++;
                    return true;
                });

        NativeMethods.setPermissionUpdaterForTests((sessionId, scope, granted) -> {
            native_.add(scope + ":" + granted);
            return true;
        });
        try {
            sink.setScope("scope.camera", true);
        } finally {
            NativeMethods.resetPermissionUpdaterForTests();
        }

        assertEquals("the update must reach native once",
                Collections.singletonList("scope.camera:true"), native_);
        assertEquals("a success reports no failure", 0, reported.size());
        assertEquals("a success schedules no terminal close", 0, scheduled[0]);
        assertNotNull("the scope is usable afterwards", gate.register(151, "scope.camera"));
    }

    @Test
    public void nativeGrantFailureReportsSchedulesCloseThrowsAndLeavesDenied() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(Admission.ADMITTED, gate.admit(101));
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
        assertEquals(Admission.ADMITTED, gate.admit(102));
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
        assertEquals(Admission.ADMITTED, gate.admit(103));
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
