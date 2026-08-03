package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import org.junit.Test;

public final class PermissionOperationGateTest {
    /**
     * Budget for asserting that a transition stays blocked behind a lease. A correct drain
     * can never release it, so only a regression can make this observation fail.
     */
    private static final long BLOCKED_OBSERVATION_MILLIS = 200;

    @Test
    public void missingSessionRejectsUpdatesAndRegistrationUntilExplicitlyOpened() {
        PermissionOperationGate gate = new PermissionOperationGate();

        assertNotNull(gate.update(6, "scope.camera", true, () -> true).failure());
        assertNull(gate.register(6, "scope.camera"));

        assertTrue(gate.open(6));
        assertNull(gate.update(6, "scope.camera", true, () -> true).failure());
        assertNotNull(gate.register(6, "scope.camera"));
    }

    @Test
    public void closeWaitsForDeferredFrameworkEntryBeforeCancelling() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(7));
        assertNull(gate.update(7, "scope.userLocation", true, () -> true).failure());
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        PermissionOperationGate.Pending pending = gate.register(
                7, "scope.userLocation", () -> events.add("cancel"));
        assertNotNull(pending);

        CountDownLatch entered = new CountDownLatch(1);
        CountDownLatch release = new CountDownLatch(1);
        Thread framework = new Thread(() -> gate.enter(pending, () -> {
            events.add("enter");
            entered.countDown();
            await(release);
            events.add("return");
        }));
        framework.start();
        assertTrue(entered.await(1, TimeUnit.SECONDS));

        CountDownLatch closeAttempting = new CountDownLatch(1);
        PermissionOperationGate.Result[] closed = {null};
        Thread close = new Thread(() -> {
            closeAttempting.countDown();
            closed[0] = gate.close(7);
        });
        close.start();
        assertTrue(closeAttempting.await(1, TimeUnit.SECONDS));

        release.countDown();
        joinBounded(framework);
        joinBounded(close);

        assertNull(closed[0].failure());
        assertEquals(Arrays.asList("enter", "return", "cancel"), events);
        assertFalse(gate.enter(pending, () -> events.add("late")));
    }

    @Test
    public void updateWaitingBehindCloseCannotReopenTheSession() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(8));
        assertNull(gate.update(8, "scope.userLocation", true, () -> true).failure());
        CountDownLatch cancelling = new CountDownLatch(1);
        CountDownLatch releaseCancellation = new CountDownLatch(1);
        gate.register(8, "scope.userLocation", () -> {
            cancelling.countDown();
            await(releaseCancellation);
        });

        PermissionOperationGate.Result[] closed = {null};
        Thread close = new Thread(() -> closed[0] = gate.close(8));
        close.start();
        assertTrue(cancelling.await(1, TimeUnit.SECONDS));

        CountDownLatch updateAttempting = new CountDownLatch(1);
        PermissionOperationGate.Result[] updated = {null};
        int[] nativeUpdates = {0};
        Thread update = new Thread(() -> {
            updateAttempting.countDown();
            updated[0] = gate.update(8, "scope.camera", true, () -> {
                nativeUpdates[0]++;
                return true;
            });
        });
        update.start();
        assertTrue(updateAttempting.await(1, TimeUnit.SECONDS));

        releaseCancellation.countDown();
        joinBounded(close);
        joinBounded(update);

        assertNull(closed[0].failure());
        assertNotNull(updated[0].failure());
        assertEquals(0, nativeUpdates[0]);
        assertFalse(gate.open(8));
        assertNull(gate.register(8, "scope.camera"));
    }

    @Test
    public void failedCloseCancellationIsRetainedAndRetriedAcrossAllScopes() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(9));
        assertNull(gate.update(9, "scope.userLocation", true, () -> true).failure());
        assertNull(gate.update(9, "scope.camera", true, () -> true).failure());
        int[] locationAttempts = {0};
        int[] cameraAttempts = {0};
        PermissionOperationGate.Pending location = gate.register(
                9,
                "scope.userLocation",
                () -> {
                    locationAttempts[0]++;
                    if (locationAttempts[0] == 1) {
                        throw new IllegalStateException("location cancel failed");
                    }
                });
        gate.register(9, "scope.camera", () -> cameraAttempts[0]++);

        PermissionOperationGate.Result first = gate.close(9);
        assertNotNull(first.failure());
        assertEquals(1, locationAttempts[0]);
        assertEquals(1, cameraAttempts[0]);
        assertFalse(gate.enter(location, () -> {}));

        PermissionOperationGate.Result retry = gate.close(9);
        assertNull(retry.failure());
        assertEquals(2, locationAttempts[0]);
        assertEquals(1, cameraAttempts[0]);
    }

    @Test
    public void nativeGrantFailureLeavesScopeDenied() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(10));

        PermissionOperationGate.Result result =
                gate.update(10, "scope.camera", true, () -> false);

        assertNotNull(result.failure());
        assertNull(gate.register(10, "scope.camera"));
    }

    @Test
    public void closeLeavesTombstoneAndCannotRetainGrantOrBeReopened() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(11));
        assertNull(gate.update(11, "scope.camera", true, () -> true).failure());

        assertNull(gate.close(11).failure());

        assertFalse(gate.open(11));
        assertNotNull(gate.update(11, "scope.camera", true, () -> true).failure());
        assertNull(gate.register(11, "scope.camera"));
    }

    @Test
    public void duplicateOpenCannotRepublishAnExistingSession() {
        PermissionOperationGate gate = new PermissionOperationGate();

        assertTrue(gate.open(12));
        assertFalse(gate.open(12));
    }

    @Test
    public void nativeUpdateDoesNotHoldSessionMonitorWhileWaitingForHostLock() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(13));
        Object hostLock = new Object();
        CountDownLatch nativeUpdateStarted = new CountDownLatch(1);
        CountDownLatch javaEntryStarted = new CountDownLatch(1);
        PermissionOperationGate.Result[] updated = {null};
        PermissionOperationGate.Pending[] registered = {null};

        Thread update = new Thread(() -> updated[0] = gate.update(
                13,
                "scope.camera",
                true,
                () -> {
                    nativeUpdateStarted.countDown();
                    await(javaEntryStarted);
                    synchronized (hostLock) {
                        return true;
                    }
                }));
        update.setDaemon(true);
        update.start();
        assertTrue(nativeUpdateStarted.await(1, TimeUnit.SECONDS));

        Thread protectedCall = new Thread(() -> {
            synchronized (hostLock) {
                javaEntryStarted.countDown();
                registered[0] = gate.register(13, "scope.camera");
            }
        });
        protectedCall.setDaemon(true);
        protectedCall.start();
        assertTrue(javaEntryStarted.await(1, TimeUnit.SECONDS));

        update.join(1000);
        protectedCall.join(1000);

        assertFalse("permission update retained the Java session monitor across native code",
                update.isAlive());
        assertFalse("protected JNI call retained the native host lock while entering Java",
                protectedCall.isAlive());
        assertNotNull(updated[0]);
        assertNull(updated[0].failure());
        assertNull(registered[0]);
        assertNotNull(gate.register(13, "scope.camera"));
    }

    @Test
    public void successfulCloseReclaimsSessionsWithoutAllowingIdReuse() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        for (int sessionId = 1000; sessionId < 2000; sessionId++) {
            assertTrue(gate.open(sessionId));
            assertNull(gate.close(sessionId).failure());
        }
        assertEquals(0, gate.retainedSessionCountForTests());
        assertFalse(gate.open(1500));

        assertTrue(gate.open(2000));
        assertNull(gate.update(2000, "scope.camera", true, () -> true).failure());
        CountDownLatch cancellationStarted = new CountDownLatch(1);
        CountDownLatch releaseCancellation = new CountDownLatch(1);
        gate.register(2000, "scope.camera", () -> {
            cancellationStarted.countDown();
            await(releaseCancellation);
        });
        Thread close = new Thread(() -> gate.close(2000));
        close.start();
        assertTrue(cancellationStarted.await(1, TimeUnit.SECONDS));

        assertFalse(gate.open(2000));
        releaseCancellation.countDown();
        joinBounded(close);

        assertEquals(0, gate.retainedSessionCountForTests());
        assertFalse(gate.open(2000));
    }

    @Test
    public void denialDrainsAdmittedScopeRunBeforeNativeUpdateAndRejectsLaterRun()
            throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2001));
        assertNull(gate.update(2001, "scope.bluetooth", true, () -> true).failure());
        assertNull(gate.update(2001, "scope.camera", true, () -> true).failure());
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        CountDownLatch callbackEntered = new CountDownLatch(1);
        CountDownLatch releaseCallback = new CountDownLatch(1);
        boolean[] admitted = {false};

        Thread callback = new Thread(() -> admitted[0] = gate.runIfGranted(
                2001,
                "scope.bluetooth",
                () -> {
                    callbackEntered.countDown();
                    await(releaseCallback);
                    events.add("callback");
                    return true;
                }));
        callback.start();
        assertTrue(callbackEntered.await(1, TimeUnit.SECONDS));

        CountDownLatch unrelatedRegistrationFinished = new CountDownLatch(1);
        PermissionOperationGate.Pending[] unrelated = {null};
        Thread registration = new Thread(() -> {
            unrelated[0] = gate.register(2001, "scope.camera");
            unrelatedRegistrationFinished.countDown();
        });
        registration.start();
        assertTrue("external callback work retained the permission-session monitor",
                unrelatedRegistrationFinished.await(1, TimeUnit.SECONDS));
        assertNotNull(unrelated[0]);
        gate.finish(unrelated[0]);

        CountDownLatch nativeUpdateEntered = new CountDownLatch(1);
        CountDownLatch releaseNativeUpdate = new CountDownLatch(1);
        CountDownLatch denialStarted = new CountDownLatch(1);
        PermissionOperationGate.Result[] denied = {null};
        Thread denial = new Thread(() -> {
            denialStarted.countDown();
            denied[0] = gate.update(2001, "scope.bluetooth", false, () -> {
                events.add("native-update");
                nativeUpdateEntered.countDown();
                await(releaseNativeUpdate);
                return true;
            });
        });
        denial.start();
        assertTrue(denialStarted.await(1, TimeUnit.SECONDS));
        assertFalse(
                "denial reached the native update while an admitted scope run held its lease",
                nativeUpdateEntered.await(BLOCKED_OBSERVATION_MILLIS, TimeUnit.MILLISECONDS));

        releaseCallback.countDown();
        joinBounded(callback);
        try {
            assertTrue(nativeUpdateEntered.await(1, TimeUnit.SECONDS));
            assertEquals(Arrays.asList("callback", "native-update"), events);
            assertFalse(gate.runIfGranted(
                    2001, "scope.bluetooth", () -> {
                        events.add("late");
                        return true;
                    }));
        } finally {
            releaseNativeUpdate.countDown();
        }
        joinBounded(denial);
        joinBounded(registration);

        assertTrue(admitted[0]);
        assertNull(denied[0].failure());
        assertEquals(Arrays.asList("callback", "native-update"), events);
    }

    @Test
    public void closeDrainsAdmittedScopeRunBeforeCancellationAndRejectsLaterRun()
            throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2002));
        assertNull(gate.update(2002, "scope.bluetooth", true, () -> true).failure());
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        CountDownLatch cancellationEntered = new CountDownLatch(1);
        assertNotNull(gate.register(2002, "scope.bluetooth", () -> {
            events.add("cancel");
            cancellationEntered.countDown();
        }));
        CountDownLatch callbackEntered = new CountDownLatch(1);
        CountDownLatch releaseCallback = new CountDownLatch(1);
        Thread callback = new Thread(() -> gate.runIfGranted(
                2002,
                "scope.bluetooth",
                () -> {
                    callbackEntered.countDown();
                    await(releaseCallback);
                    events.add("callback");
                    return true;
                }));
        callback.start();
        assertTrue(callbackEntered.await(1, TimeUnit.SECONDS));

        PermissionOperationGate.Result[] closed = {null};
        CountDownLatch closeStarted = new CountDownLatch(1);
        Thread close = new Thread(() -> {
            closeStarted.countDown();
            closed[0] = gate.close(2002);
        });
        close.start();
        assertTrue(closeStarted.await(1, TimeUnit.SECONDS));
        assertFalse(
                "close reached cancellation while an admitted scope run held its lease",
                cancellationEntered.await(BLOCKED_OBSERVATION_MILLIS, TimeUnit.MILLISECONDS));
        releaseCallback.countDown();
        joinBounded(callback);
        assertTrue(cancellationEntered.await(1, TimeUnit.SECONDS));
        joinBounded(close);

        assertNull(closed[0].failure());
        assertEquals(Arrays.asList("callback", "cancel"), events);
        assertFalse(gate.runIfGranted(2002, "scope.bluetooth", () -> true));
        assertEquals(0, gate.retainedSessionCountForTests());
        assertFalse(gate.open(2002));
    }

    @Test
    public void callbackFailureReleasesScopeRunLeaseForSubsequentDenial()
            throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2003));
        assertNull(gate.update(2003, "scope.bluetooth", true, () -> true).failure());
        IllegalStateException callbackFailure =
                new IllegalStateException("callback failed");

        try {
            gate.runIfGranted(2003, "scope.bluetooth", () -> {
                throw callbackFailure;
            });
            throw new AssertionError("callback failure was swallowed");
        } catch (IllegalStateException failure) {
            assertTrue(failure == callbackFailure);
        }

        CountDownLatch nativeUpdateEntered = new CountDownLatch(1);
        PermissionOperationGate.Result[] denied = {null};
        Thread denial = new Thread(() -> denied[0] = gate.update(
                2003,
                "scope.bluetooth",
                false,
                () -> {
                    nativeUpdateEntered.countDown();
                    return true;
                }));
        denial.setDaemon(true);
        denial.start();

        assertTrue("callback exception retained its permission lease",
                nativeUpdateEntered.await(1, TimeUnit.SECONDS));
        denial.join(1000);
        assertFalse("denial remained blocked after the callback exception", denial.isAlive());
        assertNotNull(denied[0]);
        assertNull(denied[0].failure());
        assertFalse(gate.runIfGranted(2003, "scope.bluetooth", () -> true));
    }

    private static void joinBounded(Thread thread) throws InterruptedException {
        thread.join(TimeUnit.SECONDS.toMillis(10));
        assertFalse("thread did not finish: " + thread.getName(), thread.isAlive());
    }

    private static void await(CountDownLatch latch) {
        try {
            latch.await();
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError(interrupted);
        }
    }
}
