package com.migo.runtime.internal;

import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

/**
 * The negative control for {@link AllocationProbe}.
 *
 * <p>Every gate this probe backs asserts an <em>absence</em>, and an instrument
 * that cannot see anything satisfies an absence perfectly. The probe therefore
 * performs a known allocation and refuses to proceed if the counter does not
 * report it -- and that refusal needs its own test, because mutation says so:
 * deleting the self-check left all six notification gates green. A guard whose
 * removal changes nothing is not a guard.
 *
 * <p>Switching the counter off is how a silent instrument is manufactured here.
 * It is process-wide, which is why the probe serialises bursts and why this test
 * holds that same monitor for the whole window.
 */
public final class AllocationProbeControlTest {
    private static volatile byte[] sink;

    @Test
    public void aSilentCounterFailsTheBurstInsteadOfReportingZero() {
        // Resolved while the counter still works: the probe caches it once, and
        // this test is about a counter that goes quiet, not one that is missing.
        AllocationProbe.resolveForControl();

        synchronized (AllocationProbe.ONE_BURST_AT_A_TIME) {
            AllocationProbe.setCountingEnabledForControl(false);
            try {
                AllocationProbe.assertNoSteadyStateAllocation(
                        "control: a body that certainly allocates", 2, 8,
                        () -> sink = new byte[2048]);
                fail("a burst whose instrument reports nothing must refuse, not pass");
            } catch (AssertionError expected) {
                assertTrue(
                        "the refusal must name the instrument, not the path: "
                                + expected.getMessage(),
                        expected.getMessage().contains("would mean nothing"));
            } finally {
                AllocationProbe.setCountingEnabledForControl(true);
            }
        }
    }

    /** With the counter working again, the same body is caught as an allocation. */
    @Test
    public void aWorkingCounterCatchesTheSameBody() {
        try {
            AllocationProbe.assertNoSteadyStateAllocation(
                    "control: a body that certainly allocates", 2, 8,
                    () -> sink = new byte[2048]);
            fail("an allocating body must fail its burst");
        } catch (AssertionError expected) {
            assertTrue(
                    "the failure must name the path's allocation: " + expected.getMessage(),
                    expected.getMessage().contains("byte(s) allocated over 8"));
        }
    }
}
