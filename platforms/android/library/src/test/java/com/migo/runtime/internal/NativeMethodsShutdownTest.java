package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

import org.junit.Test;

public final class NativeMethodsShutdownTest {
    @Test
    public void nativeShutdownResultRemainsObservableAndRetryable() {
        int[] attempts = {0};
        NativeMethods.setSessionShutdownForTests(sessionId -> ++attempts[0] > 1);
        try {
            assertFalse(NativeMethods.shutdown(77));
            assertTrue(NativeMethods.shutdown(77));
        } finally {
            NativeMethods.resetSessionShutdownForTests();
        }
        assertEquals(2, attempts[0]);
    }

    @Test
    public void invalidSessionDoesNotEnterNativeShutdown() {
        int[] attempts = {0};
        NativeMethods.setSessionShutdownForTests(sessionId -> {
            attempts[0]++;
            return true;
        });
        try {
            assertFalse(NativeMethods.shutdown(-1));
        } finally {
            NativeMethods.resetSessionShutdownForTests();
        }
        assertEquals(0, attempts[0]);
    }

    /**
     * The argument guard on the permission bridge, one clause at a time.
     *
     * Mutation testing negated every clause of
     * `sessionId >= 0 && scope != null && !scope.isEmpty()` and killed nothing: this
     * class only covered shutdown. The clauses are not interchangeable -- a null or empty
     * scope reaching native would record a standing decision under no scope at all, and
     * a negative id is the sentinel a failed session start returns.
     *
     * Session id 0 is asserted deliberately. It is a valid id, and it is the only case
     * that tells `>= 0` apart from `> 0`.
     */
    @Test
    public void thePermissionBridgeAdmitsOnlyAWholeRequest() {
        List<String> reached = new ArrayList<>();
        NativeMethods.setPermissionUpdaterForTests((sessionId, scope, granted) -> {
            reached.add(sessionId + ":" + scope + ":" + granted);
            return granted;
        });
        try {
            assertFalse("a negative id is refused",
                    NativeMethods.updatePermission(-1, "scope.camera", true));
            assertFalse("a null scope is refused",
                    NativeMethods.updatePermission(1, null, true));
            assertFalse("an empty scope is refused",
                    NativeMethods.updatePermission(1, "", true));
            assertTrue("a refused request must not reach native", reached.isEmpty());

            assertTrue("session id 0 is a valid id",
                    NativeMethods.updatePermission(0, "scope.camera", true));
            assertEquals(Collections.singletonList("0:scope.camera:true"), reached);

            // The verdict is the updater's, not this method's.
            assertFalse("a denial is reported as the updater reported it",
                    NativeMethods.updatePermission(0, "scope.camera", false));
        } finally {
            NativeMethods.resetPermissionUpdaterForTests();
        }
    }

    /** Session id 0 is a valid id here too, which is what pins `>= 0` against `> 0`. */
    @Test
    public void sessionZeroCanBeShutDown() {
        List<Integer> attempts = new ArrayList<>();
        NativeMethods.setSessionShutdownForTests(sessionId -> {
            attempts.add(sessionId);
            return true;
        });
        try {
            assertTrue(NativeMethods.shutdown(0));
        } finally {
            NativeMethods.resetSessionShutdownForTests();
        }
        assertEquals(Collections.singletonList(0), attempts);
    }
}
