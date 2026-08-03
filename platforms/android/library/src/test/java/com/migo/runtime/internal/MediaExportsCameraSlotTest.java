package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;

import org.junit.Test;

public final class MediaExportsCameraSlotTest {
    @Test
    public void replacementFailureKeepsOldCameraAndDestroysUnpublishedReplacement() {
        FakeCamera oldCamera = new FakeCamera();
        oldCamera.failDestroyOnce = true;
        FakeCamera replacement = new FakeCamera();
        MediaExports.CameraSlot<FakeCamera> slot =
                new MediaExports.CameraSlot<>(FakeCamera::destroy);
        slot.replace(oldCamera);

        RuntimeException failure = null;
        try {
            slot.replace(replacement);
        } catch (RuntimeException caught) {
            failure = caught;
        }

        assertEquals("destroy failed", failure.getMessage());
        assertSame(oldCamera, slot.get());
        assertEquals(1, oldCamera.destroyAttempts);
        assertEquals(1, replacement.destroyAttempts);

        FakeCamera retryReplacement = new FakeCamera();
        slot.replace(retryReplacement);

        assertSame(retryReplacement, slot.get());
        assertEquals(2, oldCamera.destroyAttempts);
    }

    @Test
    public void destroyFailureKeepsSameCameraForRetry() {
        FakeCamera camera = new FakeCamera();
        camera.failDestroyOnce = true;
        MediaExports.CameraSlot<FakeCamera> slot =
                new MediaExports.CameraSlot<>(FakeCamera::destroy);
        slot.replace(camera);

        RuntimeException failure = null;
        try {
            slot.destroy();
        } catch (RuntimeException caught) {
            failure = caught;
        }

        assertEquals("destroy failed", failure.getMessage());
        assertSame(camera, slot.get());

        slot.destroy();

        assertEquals(null, slot.get());
        assertEquals(2, camera.destroyAttempts);
    }

    private static final class FakeCamera {
        boolean failDestroyOnce;
        int destroyAttempts;

        void destroy() {
            destroyAttempts++;
            if (failDestroyOnce) {
                failDestroyOnce = false;
                throw new IllegalStateException("destroy failed");
            }
        }
    }
}
