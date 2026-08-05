package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

import org.junit.Before;
import org.junit.Test;

/**
 * The arbitration that makes camera, microphone and Bluetooth adapter ownership a
 * decision rather than a race. Every property here was previously untested because
 * no arbitration existed: two Sessions each acquired the device and the second
 * silently broke the first.
 */
public final class ExclusiveDeviceArbiterTest {

    @Before
    public void reset() {
        ExclusiveDeviceArbiter.resetForTests();
    }

    @Test
    public void theFirstSessionWinsAndTheSecondIsRefused() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));
        assertFalse(
                "a second session must not be handed a device the first is using",
                ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 2));
        assertEquals(
                Integer.valueOf(1),
                ExclusiveDeviceArbiter.ownerForTests(ExclusiveDeviceArbiter.MICROPHONE));
    }

    @Test
    public void releasingLetsTheWaitingSessionAcquire() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));
        assertFalse(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 2));

        ExclusiveDeviceArbiter.release(ExclusiveDeviceArbiter.MICROPHONE, 1);

        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 2));
        assertEquals(
                Integer.valueOf(2),
                ExclusiveDeviceArbiter.ownerForTests(ExclusiveDeviceArbiter.MICROPHONE));
    }

    /** A non-owner's release must not hand the device away from its owner. */
    @Test
    public void aNonOwnerCannotReleaseTheDevice() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));

        ExclusiveDeviceArbiter.release(ExclusiveDeviceArbiter.MICROPHONE, 2);

        assertEquals(
                Integer.valueOf(1),
                ExclusiveDeviceArbiter.ownerForTests(ExclusiveDeviceArbiter.MICROPHONE));
        assertFalse(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 2));
    }

    /**
     * Two Sessions using different cameras is legitimate: a device has several, and
     * CameraManager is built per camera id. Keying on the device class alone would
     * refuse this.
     */
    @Test
    public void differentCamerasAreIndependentlyOwnable() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("front"), 1));
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("back"), 2));

        assertFalse(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("front"), 2));
        assertFalse(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("back"), 1));
    }

    /** Re-acquiring what you already hold succeeds and needs no extra release. */
    @Test
    public void reacquiringIsIdempotentForTheOwner() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));

        ExclusiveDeviceArbiter.release(ExclusiveDeviceArbiter.MICROPHONE, 1);

        assertNull(ExclusiveDeviceArbiter.ownerForTests(ExclusiveDeviceArbiter.MICROPHONE));
    }

    /**
     * A Session torn down while holding devices must not keep every other game off
     * them for the life of the process, and teardown cannot rely on each manager
     * having released cleanly -- a failed release may be why it is being torn down.
     */
    @Test
    public void tearingDownASessionFreesEverythingItHeld() {
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 1));
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("front"), 1));
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(
                ExclusiveDeviceArbiter.BLUETOOTH_ADAPTER, 2));

        ExclusiveDeviceArbiter.releaseAll(1);

        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.MICROPHONE, 2));
        assertTrue(ExclusiveDeviceArbiter.tryAcquire(ExclusiveDeviceArbiter.camera("front"), 2));
        assertEquals(
                "another session's holdings must survive",
                Integer.valueOf(2),
                ExclusiveDeviceArbiter.ownerForTests(ExclusiveDeviceArbiter.BLUETOOTH_ADAPTER));
    }

    /**
     * Exactly one of two Sessions racing for the same device may win. A
     * check-then-act arbiter passes the sequential tests above and fails this one.
     */
    @Test
    public void exactlyOneOfTwoRacingSessionsWins() throws InterruptedException {
        for (int round = 0; round < 200; round++) {
            ExclusiveDeviceArbiter.resetForTests();
            CountDownLatch start = new CountDownLatch(1);
            AtomicInteger winners = new AtomicInteger();
            List<Thread> threads = new ArrayList<>();
            for (int session = 1; session <= 2; session++) {
                int sessionId = session;
                Thread thread = new Thread(() -> {
                    try {
                        start.await();
                    } catch (InterruptedException interrupted) {
                        Thread.currentThread().interrupt();
                        return;
                    }
                    if (ExclusiveDeviceArbiter.tryAcquire(
                            ExclusiveDeviceArbiter.MICROPHONE, sessionId)) {
                        winners.incrementAndGet();
                    }
                }, "arbiter-race-" + sessionId);
                threads.add(thread);
                thread.start();
            }
            start.countDown();
            for (Thread thread : threads) {
                thread.join(TimeUnit.SECONDS.toMillis(10));
                assertFalse("thread did not finish", thread.isAlive());
            }
            assertEquals("round " + round, 1, winners.get());
        }
    }
}
