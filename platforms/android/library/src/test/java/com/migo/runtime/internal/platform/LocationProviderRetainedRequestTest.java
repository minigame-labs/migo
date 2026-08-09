package com.migo.runtime.internal.platform;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import com.migo.runtime.internal.PermissionOperationGate;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import com.migo.runtime.internal.PermissionOperationGate.Admission;
import org.junit.Test;

public final class LocationProviderRetainedRequestTest {
    @Test
    public void callbackFinishesAndDeliversOnlyAfterBothRemovalsSucceed() {
        Removal removal = new Removal();
        List<String> delivered = new ArrayList<>();
        int[] finished = {0};
        List<RuntimeException> failures = new ArrayList<>();
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                removal,
                () -> finished[0]++,
                delivered::add,
                failures::add);

        request.complete("callback");

        assertTrue(request.isReleased());
        assertEquals(1, removal.listenerAttempts);
        assertEquals(1, removal.timeoutAttempts);
        assertEquals(1, finished[0]);
        assertEquals(Arrays.asList("callback"), delivered);
        assertEquals(0, failures.size());
    }

    @Test
    public void timeoutRemovalFailureRetainsRequestForExplicitCloseRetry() {
        Removal removal = new Removal();
        removal.failTimeoutOnce = true;
        List<String> delivered = new ArrayList<>();
        int[] finished = {0};
        List<RuntimeException> failures = new ArrayList<>();
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                removal,
                () -> finished[0]++,
                delivered::add,
                failures::add);

        request.complete("timeout");

        assertFalse(request.isReleased());
        assertEquals(1, removal.listenerAttempts);
        assertEquals(1, removal.timeoutAttempts);
        assertEquals(0, finished[0]);
        assertEquals(new ArrayList<>(), delivered);
        assertEquals(1, failures.size());

        request.cancel("close fallback");

        assertTrue(request.isReleased());
        assertEquals(1, removal.listenerAttempts);
        assertEquals(2, removal.timeoutAttempts);
        assertEquals(1, finished[0]);
        assertEquals(Arrays.asList("close fallback"), delivered);
    }

    @Test
    public void listenerRemovalFailureRetainsHandleUntilRevokeRetriesIt() {
        Removal removal = new Removal();
        removal.failListenerOnce = true;
        List<String> delivered = new ArrayList<>();
        int[] finished = {0};
        List<RuntimeException> failures = new ArrayList<>();
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                removal,
                () -> finished[0]++,
                delivered::add,
                failures::add);

        request.complete("callback");
        assertFalse(request.isReleased());
        assertEquals(1, failures.size());

        request.cancel("revoke fallback");

        assertTrue(request.isReleased());
        assertEquals(2, removal.listenerAttempts);
        assertEquals(1, removal.timeoutAttempts);
        assertEquals(1, finished[0]);
        assertEquals(Arrays.asList("revoke fallback"), delivered);
    }

    @Test
    public void callbackAndRevokeCloseInterleavingDeliversOnlyFirstResultOnce() throws Exception {
        CountDownLatch listenerRemovalStarted = new CountDownLatch(1);
        CountDownLatch releaseListenerRemoval = new CountDownLatch(1);
        LocationProvider.RequestRemoval removal = new LocationProvider.RequestRemoval() {
            @Override public void removeListener() {
                listenerRemovalStarted.countDown();
                await(releaseListenerRemoval);
            }

            @Override public void removeTimeout() {}
        };
        List<String> delivered = new ArrayList<>();
        int[] finished = {0};
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                removal,
                () -> finished[0]++,
                delivered::add,
                failure -> { throw new AssertionError(failure); });

        Thread callback = new Thread(() -> request.complete("callback"));
        callback.start();
        assertTrue(listenerRemovalStarted.await(1, TimeUnit.SECONDS));

        RuntimeException[] cancelFailure = {null};
        Thread revoke = new Thread(() -> {
            try {
                request.cancel("revoke fallback");
            } catch (RuntimeException failure) {
                cancelFailure[0] = failure;
            }
        });
        revoke.setDaemon(true);
        revoke.start();
        // Parked inside the in-flight cleanup, not merely started. A latch this
        // thread counts down before calling `cancel` proves only that it was
        // scheduled once: the release below could then land before it reaches the
        // monitor, the callback would win, and the assertion at the bottom would
        // fail for a reason that has nothing to do with the property. That is a
        // race in the test, and it is what turned this red on CI while passing
        // every local run. The two sibling interleaving cases here already wait
        // for the same state; this one did not.
        awaitParkedOnInFlightCleanup(revoke);

        releaseListenerRemoval.countDown();
        callback.join();
        revoke.join();

        assertTrue(request.isReleased());
        assertEquals(null, cancelFailure[0]);
        assertEquals(1, finished[0]);
        assertEquals(Arrays.asList("revoke fallback"), delivered);
    }

    @Test
    public void callbackCleanupAndGateCloseDoNotInvertRetainedAndSessionLocks() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(Admission.ADMITTED, gate.admit(77));
        assertEquals(null,
                gate.update(77, "scope.userLocation", true, () -> true).failure());
        CountDownLatch listenerRemovalStarted = new CountDownLatch(1);
        CountDownLatch releaseListenerRemoval = new CountDownLatch(1);
        CountDownLatch cancelStarted = new CountDownLatch(1);
        List<String> delivered = new ArrayList<>();
        PermissionOperationGate.Pending pending =
                gate.register(77, "scope.userLocation");
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                new LocationProvider.RequestRemoval() {
                    @Override public void removeListener() {
                        listenerRemovalStarted.countDown();
                        await(releaseListenerRemoval);
                    }

                    @Override public void removeTimeout() {}
                },
                () -> gate.finish(pending),
                delivered::add,
                failure -> {});
        pending.setCancellation(() -> {
            cancelStarted.countDown();
            request.cancel("revoke fallback");
        });

        Thread callback = new Thread(() -> request.complete("callback"));
        callback.setDaemon(true);
        callback.start();
        assertTrue(listenerRemovalStarted.await(1, TimeUnit.SECONDS));

        PermissionOperationGate.Result[] closed = {null};
        Thread close = new Thread(() -> closed[0] = gate.close(77));
        close.setDaemon(true);
        close.start();
        assertTrue(cancelStarted.await(1, TimeUnit.SECONDS));
        awaitParkedOnInFlightCleanup(close);

        releaseListenerRemoval.countDown();
        callback.join(1000);
        close.join(1000);

        assertFalse("callback retained the location monitor while taking the session lock",
                callback.isAlive());
        assertFalse("close retained the session lock while taking the location monitor",
                close.isAlive());
        assertEquals(null, closed[0].failure());
        assertEquals(1, delivered.size());
        assertEquals("revoke fallback", delivered.get(0));
    }

    @Test
    public void inFlightCleanupFailureMakesGateCloseRetryable() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(Admission.ADMITTED, gate.admit(78));
        assertEquals(null,
                gate.update(78, "scope.userLocation", true, () -> true).failure());
        CountDownLatch listenerRemovalStarted = new CountDownLatch(1);
        CountDownLatch releaseListenerRemoval = new CountDownLatch(1);
        CountDownLatch cancelStarted = new CountDownLatch(1);
        List<String> delivered = new ArrayList<>();
        int[] listenerAttempts = {0};
        PermissionOperationGate.Pending pending =
                gate.register(78, "scope.userLocation");
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                new LocationProvider.RequestRemoval() {
                    @Override public void removeListener() {
                        listenerAttempts[0]++;
                        if (listenerAttempts[0] == 1) {
                            listenerRemovalStarted.countDown();
                            await(releaseListenerRemoval);
                            throw new IllegalStateException("listener removal failed");
                        }
                    }

                    @Override public void removeTimeout() {}
                },
                () -> gate.finish(pending),
                delivered::add,
                failure -> {});
        pending.setCancellation(() -> {
            cancelStarted.countDown();
            request.cancel("close fallback");
        });

        Thread callback = new Thread(() -> request.complete("callback"));
        callback.start();
        assertTrue(listenerRemovalStarted.await(1, TimeUnit.SECONDS));

        PermissionOperationGate.Result[] firstClose = {null};
        Thread close = new Thread(() -> firstClose[0] = gate.close(78));
        close.start();
        assertTrue(cancelStarted.await(1, TimeUnit.SECONDS));
        awaitParkedOnInFlightCleanup(close);

        releaseListenerRemoval.countDown();
        callback.join(1000);
        close.join(1000);

        assertFalse(callback.isAlive());
        assertFalse(close.isAlive());
        assertNotNull(firstClose[0].failure());
        assertFalse(request.isReleased());
        assertEquals(0, delivered.size());

        PermissionOperationGate.Result retry = gate.close(78);

        assertEquals(null, retry.failure());
        assertTrue(request.isReleased());
        assertEquals(2, listenerAttempts[0]);
        assertEquals(Arrays.asList("close fallback"), delivered);
    }

    @Test
    public void directGateCloseFailureRetainsCancellationForRetry() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertEquals(Admission.ADMITTED, gate.admit(79));
        assertEquals(null,
                gate.update(79, "scope.userLocation", true, () -> true).failure());
        Removal removal = new Removal();
        removal.failListenerOnce = true;
        List<String> delivered = new ArrayList<>();
        List<RuntimeException> reported = new ArrayList<>();
        PermissionOperationGate.Pending pending =
                gate.register(79, "scope.userLocation");
        LocationProvider.RetainedRequest<String> request = new LocationProvider.RetainedRequest<>(
                removal,
                () -> gate.finish(pending),
                delivered::add,
                reported::add);
        pending.setCancellation(() -> request.cancel("close fallback"));

        PermissionOperationGate.Result firstClose = gate.close(79);

        assertNotNull(firstClose.failure());
        assertFalse(request.isReleased());
        assertEquals(1, removal.listenerAttempts);
        assertEquals(1, removal.timeoutAttempts);
        assertEquals(1, reported.size());
        assertEquals(0, delivered.size());

        PermissionOperationGate.Result retry = gate.close(79);

        assertEquals(null, retry.failure());
        assertTrue(request.isReleased());
        assertEquals(2, removal.listenerAttempts);
        assertEquals(1, removal.timeoutAttempts);
        assertEquals(Arrays.asList("close fallback"), delivered);
    }

    private static final class Removal implements LocationProvider.RequestRemoval {
        int listenerAttempts;
        int timeoutAttempts;
        boolean failListenerOnce;
        boolean failTimeoutOnce;

        @Override public void removeListener() {
            listenerAttempts++;
            if (failListenerOnce) {
                failListenerOnce = false;
                throw new IllegalStateException("listener removal failed");
            }
        }

        @Override public void removeTimeout() {
            timeoutAttempts++;
            if (failTimeoutOnce) {
                failTimeoutOnce = false;
                throw new IllegalStateException("timeout removal failed");
            }
        }
    }

    private static void await(CountDownLatch latch) {
        try {
            latch.await();
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError(interrupted);
        }
    }

    /**
     * Blocks until {@code closing} has actually parked inside the retained request's in-flight
     * cleanup wait.
     *
     * <p>The cancellation signals before it calls {@code cancel}, so the "cancel started" latch
     * only says the close thread reached the cancellation body — not that it reached the wait
     * whose ordering these interleaving tests describe. Releasing the blocked removal on that
     * latch alone lets the close thread be descheduled in between, the callback thread finish its
     * cleanup first, and cancel then take the ordinary path. That is correct behaviour, but it is
     * the other interleaving, so the assertions written for this one fail. Waiting for the park
     * pins the one under test.
     */
    private static void awaitParkedOnInFlightCleanup(Thread closing) {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
        while (System.nanoTime() < deadline) {
            if (closing.getState() == Thread.State.WAITING && isInRetainedRequest(closing)) {
                return;
            }
            Thread.yield();
        }
        throw new AssertionError(
                "close thread never parked on the in-flight cleanup: " + closing.getState());
    }

    private static boolean isInRetainedRequest(Thread thread) {
        for (StackTraceElement frame : thread.getStackTrace()) {
            if (frame.getClassName().endsWith("RetainedRequest")) {
                return true;
            }
        }
        return false;
    }
}
