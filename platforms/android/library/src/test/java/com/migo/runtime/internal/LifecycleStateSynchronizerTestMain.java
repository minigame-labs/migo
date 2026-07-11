package com.migo.runtime.internal;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

public final class LifecycleStateSynchronizerTestMain {
    public static void main(String[] args) throws Exception {
        lifecycleTransitionWinsOverStaleFactorySnapshot();
        System.out.println("LifecycleStateSynchronizer tests passed");
    }

    private static void lifecycleTransitionWinsOverStaleFactorySnapshot() throws Exception {
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

        assertFalse(factorySync.isAlive());
        assertFalse(lifecycle.isAlive());
        assertTrue(managerSuspended.get());
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

    private static void assertTrue(boolean value) {
        if (!value) throw new AssertionError("expected true");
    }

    private static void assertFalse(boolean value) {
        if (value) throw new AssertionError("expected false");
    }

    private LifecycleStateSynchronizerTestMain() {}
}
