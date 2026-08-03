package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

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
}
