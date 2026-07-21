package com.migo.runtime.internal;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;

/**
 * Host-JVM test for the factory-snapshot versus lifecycle-transition race.
 *
 * <p>The interleaving is forced with latches rather than sleeps: the factory
 * thread is held inside the snapshot read until the lifecycle thread has already
 * moved the session to suspended, which is the exact ordering the synchronizer
 * exists to survive.
 */
public final class LifecycleStateSynchronizerTest {
    @Test(timeout = 10_000)
    public void lifecycleTransitionWinsOverStaleFactorySnapshot() throws Exception {
        Object managerMonitor = new Object();
        AtomicBoolean sessionSuspended = new AtomicBoolean(false);
        AtomicBoolean managerSuspended = new AtomicBoolean(false);
        CountDownLatch snapshotRead = new CountDownLatch(1);
        CountDownLatch lifecycleStateChanged = new CountDownLatch(1);
        CountDownLatch releaseSnapshot = new CountDownLatch(1);

        Thread factorySync = new Thread(() -> LifecycleStateSynchronizer.synchronize(
                managerMonitor,
                () -> {
                    boolean snapshot = sessionSuspended.get();
                    snapshotRead.countDown();
                    await(releaseSnapshot);
                    return snapshot;
                },
                managerSuspended::set));

        Thread lifecycle = new Thread(() -> {
            await(snapshotRead);
            sessionSuspended.set(true);
            lifecycleStateChanged.countDown();
            synchronized (managerMonitor) {
                managerSuspended.set(true);
            }
        });

        factorySync.start();
        lifecycle.start();
        assertTrue(lifecycleStateChanged.await(2, TimeUnit.SECONDS));
        releaseSnapshot.countDown();
        factorySync.join(2000);
        lifecycle.join(2000);

        assertFalse("factory sync thread did not finish", factorySync.isAlive());
        assertFalse("lifecycle thread did not finish", lifecycle.isAlive());
        assertTrue("the later lifecycle transition must win", managerSuspended.get());
    }

    private static void await(CountDownLatch latch) {
        try {
            if (!latch.await(2, TimeUnit.SECONDS)) {
                throw new AssertionError("timed out waiting for test interleaving");
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError("interrupted", e);
        }
    }
}
