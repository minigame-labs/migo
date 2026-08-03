package com.migo.runtime;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class ErrorCodeTest {
    @Test
    public void inputSaturationIsStableAndRecoverable() {
        assertEquals(11, ErrorCode.NATIVE_INPUT_SATURATED);
        assertEquals("Input transport saturated",
                ErrorCode.getMessage(ErrorCode.NATIVE_INPUT_SATURATED));
        assertTrue(ErrorCode.isRecoverable(ErrorCode.NATIVE_INPUT_SATURATED));
    }
}
