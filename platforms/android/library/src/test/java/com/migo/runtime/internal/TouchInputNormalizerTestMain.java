package com.migo.runtime.internal;

/** Host-JVM regression tests for touch scalar normalization. */
public final class TouchInputNormalizerTestMain {
    public static void main(String[] args) {
        expect(0.0f, TouchInputNormalizer.pressure(0.0f), "zero is a valid pressure");
        expect(0.25f, TouchInputNormalizer.pressure(0.25f), "fraction is preserved");
        expect(1.0f, TouchInputNormalizer.pressure(1.0f), "one is preserved");
        expect(0.0f, TouchInputNormalizer.pressure(-0.1f), "negative clamps low");
        expect(1.0f, TouchInputNormalizer.pressure(1.5f), "large value clamps high");
        expect(0.0f, TouchInputNormalizer.pressure(Float.NaN), "NaN fails closed");
        expect(1.0f, TouchInputNormalizer.pressure(Float.POSITIVE_INFINITY), "infinity clamps high");
        System.out.println("TouchInputNormalizer tests passed");
    }

    private static void expect(float expected, float actual, String message) {
        if (Float.floatToIntBits(expected) != Float.floatToIntBits(actual)) {
            throw new AssertionError(message + ": expected=" + expected + ", actual=" + actual);
        }
    }

    private TouchInputNormalizerTestMain() {}
}
