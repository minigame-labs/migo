package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

/**
 * Host-JVM regression tests for touch scalar normalization.
 *
 * <p>Compared with bit patterns rather than a delta: the point of every case is
 * that the result is one of the two exact endpoints or the input unchanged, and
 * a tolerance would let a wrong-but-close value pass. It also makes the NaN case
 * meaningful, since {@code NaN != NaN} under any delta comparison.
 */
public final class TouchInputNormalizerTest {
    private static void expect(float expected, float actual, String message) {
        assertEquals(message, Float.floatToIntBits(expected), Float.floatToIntBits(actual));
    }

    @Test
    public void pressureKeepsValuesInsideTheUnitRange() {
        expect(0.0f, TouchInputNormalizer.pressure(0.0f), "zero is a valid pressure");
        expect(0.25f, TouchInputNormalizer.pressure(0.25f), "fraction is preserved");
        expect(1.0f, TouchInputNormalizer.pressure(1.0f), "one is preserved");
    }

    @Test
    public void pressureClampsValuesOutsideTheUnitRange() {
        expect(0.0f, TouchInputNormalizer.pressure(-0.1f), "negative clamps low");
        expect(1.0f, TouchInputNormalizer.pressure(1.5f), "large value clamps high");
        expect(1.0f, TouchInputNormalizer.pressure(Float.POSITIVE_INFINITY), "infinity clamps high");
    }

    @Test
    public void pressureFailsClosedOnNaN() {
        expect(0.0f, TouchInputNormalizer.pressure(Float.NaN), "NaN fails closed");
    }
}
