package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class TerminalCleanupStateTest {
    @Test
    public void resourceFailureSkipsOwnershipReleaseAndRemainsRetryable() {
        TerminalCleanupState cleanup = new TerminalCleanupState();
        int[] targetedAttempts = {0};
        int[] otherAttempts = {0};
        int[] ownershipReleases = {0};

        TerminalCleanupState.Result firstClose = cleanup.attempt(
                () -> ResourceCleanup.runAll(
                        () -> {
                            targetedAttempts[0]++;
                            throw new IllegalStateException("resource still active");
                        },
                        () -> otherAttempts[0]++),
                () -> {},
                () -> ownershipReleases[0]++);

        assertNotNull(firstClose.failure());
        assertEquals(1, targetedAttempts[0]);
        assertEquals(1, otherAttempts[0]);
        assertEquals(0, ownershipReleases[0]);
        assertFalse(cleanup.isComplete());

        TerminalCleanupState.Result explicitRetry = cleanup.attempt(
                () -> ResourceCleanup.runAll(
                        () -> targetedAttempts[0]++,
                        () -> otherAttempts[0]++),
                () -> {},
                () -> ownershipReleases[0]++);

        assertNull(explicitRetry.failure());
        assertTrue(explicitRetry.completed());
        assertTrue(cleanup.isComplete());
        assertEquals(2, targetedAttempts[0]);
        assertEquals(2, otherAttempts[0]);
        assertEquals(1, ownershipReleases[0]);
    }

    @Test
    public void ownershipFailureRetriesWithoutMarkingCleanupComplete() {
        TerminalCleanupState cleanup = new TerminalCleanupState();
        int[] resources = {0};
        int[] ownership = {0};

        TerminalCleanupState.Result first = cleanup.attempt(
                () -> resources[0]++,
                () -> {},
                () -> {
                    ownership[0]++;
                    throw new IllegalStateException("ownership retained");
                });

        assertNotNull(first.failure());
        assertFalse(cleanup.isComplete());

        TerminalCleanupState.Result retry = cleanup.attempt(
                () -> resources[0]++,
                () -> {},
                () -> ownership[0]++);

        assertNull(retry.failure());
        assertEquals(2, resources[0]);
        assertEquals(2, ownership[0]);
        assertTrue(cleanup.isComplete());
    }

    @Test
    public void ownershipReleaseAttemptsEveryActionAndAggregatesFailure() {
        TerminalCleanupState cleanup = new TerminalCleanupState();
        int[] registry = {0};
        int[] temp = {0};
        int[] sessions = {0};

        TerminalCleanupState.Result result = cleanup.attempt(
                () -> {},
                () -> {},
                () -> {
                    registry[0]++;
                    throw new IllegalStateException("registry release failed");
                },
                () -> temp[0]++,
                () -> sessions[0]++);

        assertNotNull(result.failure());
        assertEquals(1, registry[0]);
        assertEquals(1, temp[0]);
        assertEquals(1, sessions[0]);
        assertFalse(cleanup.isComplete());
    }

    @Test
    public void shutdownFailureSkipsOwnershipReleaseAndRetriesTheWholeBarrier() {
        TerminalCleanupState cleanup = new TerminalCleanupState();
        int[] resources = {0};
        int[] shutdowns = {0};
        int[] ownership = {0};

        TerminalCleanupState.Result first = cleanup.attempt(
                () -> resources[0]++,
                () -> {
                    shutdowns[0]++;
                    throw new IllegalStateException("native join failed");
                },
                () -> ownership[0]++);

        assertNotNull(first.failure());
        assertEquals(1, resources[0]);
        assertEquals(1, shutdowns[0]);
        assertEquals(0, ownership[0]);
        assertFalse(cleanup.isComplete());

        TerminalCleanupState.Result retry = cleanup.attempt(
                () -> resources[0]++,
                () -> shutdowns[0]++,
                () -> ownership[0]++);

        assertNull(retry.failure());
        assertEquals(2, resources[0]);
        assertEquals(2, shutdowns[0]);
        assertEquals(1, ownership[0]);
        assertTrue(cleanup.isComplete());
    }
}
