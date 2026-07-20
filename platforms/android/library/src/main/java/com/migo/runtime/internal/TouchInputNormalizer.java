package com.migo.runtime.internal;

/** Scalar normalization shared by the allocation-free Android touch path. */
final class TouchInputNormalizer {
    /**
     * Web {@code Touch.force} is finite and normalized to [0, 1]. Unknown or
     * invalid pressure is zero; it must never be invented as a full press.
     */
    static float pressure(float value) {
        // `!(value > 0)` deliberately catches NaN as well as zero/negative
        // values and canonicalizes negative zero.
        if (!(value > 0.0f)) return 0.0f;
        return value < 1.0f ? value : 1.0f;
    }

    private TouchInputNormalizer() {}
}
