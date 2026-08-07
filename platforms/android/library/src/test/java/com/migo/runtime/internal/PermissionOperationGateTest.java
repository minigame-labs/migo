package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.CyclicBarrier;
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
    public void aLowerSessionIdOpenedAfterAHigherOneStillGetsItsPermissions() {
        // Session ids are allocated on the caller thread but opened from each session's own
        // thread, so two sessions starting together can arrive here in the opposite order.
        // Neither id was retired, so both must be admitted and both must stay usable.
        PermissionOperationGate gate = new PermissionOperationGate();

        assertTrue("the first session was refused", gate.open(3007));
        assertTrue(
                "a live session was refused because a higher id opened first",
                gate.open(3005));

        assertNull(gate.update(3005, "scope.camera", true, () -> true).failure());
        assertNotNull(
                "a granted scope on a live session surfaced as a denial",
                gate.register(3005, "scope.camera"));
        assertNull(gate.update(3007, "scope.camera", true, () -> true).failure());
        assertNotNull(gate.register(3007, "scope.camera"));
    }

    @Test
    public void aClosedIdStaysRetiredWhenAHigherIdOpensAfterwards() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(3011));
        assertNull(gate.close(3011).failure());

        // A later, unrelated session must not resurrect the retired id.
        assertTrue(gate.open(3012));

        assertFalse("a retired id was reopened", gate.open(3011));
        assertNotNull(gate.update(3011, "scope.camera", true, () -> true).failure());
        assertNull(gate.register(3011, "scope.camera"));
    }

    /**
     * A grant belongs to the Session that was granted it, and to no other. This gate is a
     * process-wide static keyed by session id, so two concurrent Sessions meet inside one
     * object -- and a grant that leaked between them would let one game use a capability
     * the user approved for another. That is the permission half of Section 6.4's
     * concurrent-session isolation, and it was the group task 0.21 recorded as untested.
     *
     * <p>The existing cross-session test grants a scope and then checks the granted
     * session still works. This checks the other direction, which nothing did: that the
     * session which was <em>not</em> granted is refused. Both directions are needed
     * because the first alone passes over a gate that grants everyone.
     *
     * <p>The positive assertion in the middle is that control: a gate that granted nobody
     * would satisfy both denials while breaking every permission in the product.
     */
    @Test
    public void aGrantOnOneSessionLeavesTheSameScopeDeniedOnAnother() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(4001));
        assertTrue(gate.open(4002));

        assertNull(gate.update(4001, "scope.camera", true, () -> true).failure());

        assertTrue(
                "the session the grant was made for could not use it",
                gate.runIfGranted(4001, "scope.camera", () -> true));

        assertFalse(
                "one session's grant admitted another session's callback",
                gate.runIfGranted(4002, "scope.camera", () -> true));
        assertNull(
                "one session's grant let another session register a cancellation",
                gate.register(4002, "scope.camera"));
    }

    @Test
    public void closingOneSessionLeavesAnotherLiveSessionUntouched() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(3022));
        assertTrue("a live session was refused because a higher id opened first",
                gate.open(3021));
        assertNull(gate.update(3021, "scope.camera", true, () -> true).failure());

        assertNull(gate.close(3022).failure());

        assertNotNull(
                "closing a sibling session denied a live session",
                gate.register(3021, "scope.camera"));
        assertTrue(gate.runIfGranted(3021, "scope.camera", () -> true));
        // A fresh id below the closed one is still admissible.
        assertTrue(gate.open(3020));
        assertNull(gate.update(3020, "scope.camera", true, () -> true).failure());
        assertEquals(2, gate.retainedSessionCountForTests());
    }

    @Test
    public void replacedCancellationRunsExactlyOnceAndAThrowingOneIsRetained() {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(3030));
        assertNull(gate.update(3030, "scope.userLocation", true, () -> true).failure());
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        int[] replacementRuns = {0};
        PermissionOperationGate.Pending pending = gate.register(
                3030, "scope.userLocation", () -> events.add("original"));
        assertNotNull(pending);

        // The replacement is installed from inside the framework entry, exactly as
        // LocationProvider does once the framework hands back its cancellable request.
        assertTrue(gate.enter(pending, () -> pending.setCancellation(() -> {
            replacementRuns[0]++;
            events.add("replacement");
            if (replacementRuns[0] == 1) {
                throw new IllegalStateException("replacement cancel failed");
            }
        })));

        PermissionOperationGate.Result denied =
                gate.update(3030, "scope.userLocation", false, () -> true);
        assertNotNull("a throwing cancellation was reported as success", denied.failure());
        assertEquals(
                "denial ran a cancellation other than the installed replacement",
                Collections.singletonList("replacement"),
                events);
        assertEquals(1, replacementRuns[0]);
        assertFalse(gate.enter(pending, () -> events.add("late")));

        // The throwing entry is retained, so close retries it and retires it on success.
        assertNull(gate.close(3030).failure());
        assertEquals(
                Arrays.asList("replacement", "replacement"),
                events);
        assertEquals(2, replacementRuns[0]);

        assertNull(gate.close(3030).failure());
        assertEquals(
                "a retired cancellation ran again after succeeding",
                2,
                replacementRuns[0]);
    }

    /**
     * The executed cancellation must be the one published under the session monitor when the
     * pending set was snapshotted. Cancellations run with the monitor released, so a
     * concurrent {@code setCancellation} can otherwise swap the action of a pending that has
     * been snapshotted but not yet run, and the action actually invoked stops being the one
     * the snapshot admitted.
     */
    @Test
    public void aCancellationReplacedWhileAnotherRunsDoesNotSwapTheExecutedAction()
            throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(3031));
        assertNull(gate.update(3031, "scope.userLocation", true, () -> true).failure());
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        CountDownLatch firstEntered = new CountDownLatch(1);
        CountDownLatch releaseFirst = new CountDownLatch(1);
        // Snapshotted cancellations run sequentially on the denying thread, so the first one
        // to run can park while the replacement is installed on the one still queued.
        ResourceCleanup.Action original = () -> {
            events.add("original");
            if (firstEntered.getCount() > 0) {
                firstEntered.countDown();
                await(releaseFirst);
            }
        };
        PermissionOperationGate.Pending first =
                gate.register(3031, "scope.userLocation", original);
        PermissionOperationGate.Pending second =
                gate.register(3031, "scope.userLocation", original);
        assertNotNull(first);
        assertNotNull(second);

        PermissionOperationGate.Result[] denied = {null};
        Thread denial = new Thread(
                () -> denied[0] = gate.update(3031, "scope.userLocation", false, () -> true));
        denial.start();
        assertTrue(firstEntered.await(1, TimeUnit.SECONDS));

        first.setCancellation(() -> events.add("replacement"));
        second.setCancellation(() -> events.add("replacement"));
        releaseFirst.countDown();
        joinBounded(denial);

        assertNull(denied[0].failure());
        assertEquals(
                "a cancellation installed after the pending snapshot replaced the action the"
                        + " snapshot admitted",
                Arrays.asList("original", "original"),
                events);
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

    @Test
    public void closeCancellationDoesNotRetainTheSessionMonitor() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2004));
        assertNull(gate.update(2004, "scope.bluetooth", true, () -> true).failure());
        assertNull(gate.update(2004, "scope.camera", true, () -> true).failure());
        CountDownLatch cancellationEntered = new CountDownLatch(1);
        CountDownLatch releaseCancellation = new CountDownLatch(1);
        assertNotNull(gate.register(2004, "scope.bluetooth", () -> {
            cancellationEntered.countDown();
            await(releaseCancellation);
        }));

        Thread close = new Thread(() -> gate.close(2004));
        close.start();
        assertTrue(cancellationEntered.await(1, TimeUnit.SECONDS));

        CountDownLatch observed = new CountDownLatch(1);
        Thread observer = new Thread(() -> {
            gate.register(2004, "scope.camera");
            observed.countDown();
        });
        observer.setDaemon(true);
        observer.start();

        try {
            assertTrue(
                    "a blocked cancellation retained the permission-session monitor",
                    observed.await(1, TimeUnit.SECONDS));
        } finally {
            releaseCancellation.countDown();
        }
        joinBounded(close);
        joinBounded(observer);
    }

    /**
     * Contention on the per-event admission path is a structural property rather than a
     * behavioural one: every holder of the shared session map releases it in nanoseconds,
     * so no functional fixture can tell a shared monitor apart from a concurrent map. This
     * asserts the invariant directly; the sibling test guards the concurrency it enables.
     */
    @Test
    public void perEventSessionLookupTakesNoLockSharedAcrossSessions() throws Exception {
        Field sessions = PermissionOperationGate.class.getDeclaredField("sessions");

        assertTrue(
                "per-event admission must resolve a session without a lock shared across"
                        + " sessions; declared type was " + sessions.getType().getName(),
                ConcurrentMap.class.isAssignableFrom(sessions.getType()));
    }

    @Test
    public void twoSessionsAdmitCallbacksConcurrently() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2005));
        assertNull(gate.update(2005, "scope.bluetooth", true, () -> true).failure());
        assertTrue(gate.open(2006));
        assertNull(gate.update(2006, "scope.bluetooth", true, () -> true).failure());
        CyclicBarrier bothInside = new CyclicBarrier(2);
        boolean[] admitted = {false, false};

        Thread first = new Thread(() -> admitted[0] = gate.runIfGranted(
                2005, "scope.bluetooth", () -> awaitBarrier(bothInside)));
        Thread second = new Thread(() -> admitted[1] = gate.runIfGranted(
                2006, "scope.bluetooth", () -> awaitBarrier(bothInside)));
        first.setDaemon(true);
        second.setDaemon(true);
        first.start();
        second.start();
        joinBounded(first);
        joinBounded(second);

        assertTrue(admitted[0]);
        assertTrue(admitted[1]);
    }

    private static boolean awaitBarrier(CyclicBarrier barrier) {
        try {
            barrier.await(5, TimeUnit.SECONDS);
            return true;
        } catch (Exception failure) {
            throw new AssertionError(
                    "two sessions could not hold an admitted callback at the same time",
                    failure);
        }
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
