package com.migo.runtime.internal;

import java.nio.ByteBuffer;

import org.junit.Test;

/**
 * Metadata rejected by the Java callback facade must not reach the raw JNI
 * declaration. These host-JVM tests deliberately run without libmigo loaded:
 * a native call would throw {@link UnsatisfiedLinkError}.
 */
public final class NativeMethodsCameraFrameValidationTest {

    private static ByteBuffer directPlane() {
        return ByteBuffer.allocateDirect(1);
    }

    @Test
    public void rejectsOversizedDimensionsBeforeCallingNative() {
        ByteBuffer plane = directPlane();

        NativeMethods.onCameraFrameData(1, 0L, 1,
                plane, 0, 1,
                plane, 0, 0,
                plane, 0, 0,
                8193, 1);
    }

    @Test
    public void rejectsOversizedPlanePayloadBeforeCallingNative() {
        ByteBuffer plane = directPlane();

        NativeMethods.onCameraFrameData(1, 0L, 1,
                plane, 0, 64 * 1024 * 1024 + 1,
                plane, 0, 0,
                plane, 0, 0,
                1, 1);
    }
}
