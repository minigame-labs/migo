package com.migo.runtime.internal.util;

import static org.junit.Assert.assertEquals;

import com.migo.runtime.RuntimeConfig;

import org.junit.After;
import org.junit.Test;

/**
 * One session's log level must not be able to silence another's.
 *
 * <p>The level was a single process-wide switch, and on this side of the boundary
 * nothing wrote to it at all — the only mutator had no callers, so an embedder's
 * configured level was ignored and every record was filtered at the hardcoded
 * {@code WARN}. Wiring it naively would have reproduced the engine's defect on
 * this side: last writer wins, so a second game starting with {@code OFF} silences
 * the first one that asked for {@code DEBUG}.
 *
 * <p>These records carry one tag and no session id, so they cannot be filtered per
 * session. What is asserted here is the direction that can be guaranteed: the
 * level in force is the most verbose any live session asked for, and it comes back
 * down only when that session goes away.
 */
public final class LoggerSessionLevelTest {

    @After
    public void clearSessions() {
        // Process-wide state shared with every other test in this JVM.
        for (int sessionId = 1; sessionId <= 3; sessionId++) {
            Logger.unregisterSession(sessionId);
        }
    }

    @Test
    public void aConfiguredLevelIsAppliedAtAll() {
        Logger.registerSession(1, RuntimeConfig.LogLevel.TRACE);

        assertEquals(RuntimeConfig.LogLevel.TRACE.ordinal(), Logger.effectiveLevel());
    }

    /** The defect, on this side of the boundary. */
    @Test
    public void aSecondSessionCannotSilenceALiveOne() {
        Logger.registerSession(1, RuntimeConfig.LogLevel.DEBUG);
        Logger.registerSession(2, RuntimeConfig.LogLevel.OFF);

        assertEquals(
                "a session starting with OFF silenced a live session's DEBUG",
                RuntimeConfig.LogLevel.DEBUG.ordinal(),
                Logger.effectiveLevel());
    }

    @Test
    public void closingTheVerboseSessionLetsTheLevelComeBackDown() {
        Logger.registerSession(1, RuntimeConfig.LogLevel.TRACE);
        Logger.registerSession(2, RuntimeConfig.LogLevel.ERROR);
        assertEquals(RuntimeConfig.LogLevel.TRACE.ordinal(), Logger.effectiveLevel());

        Logger.unregisterSession(1);

        assertEquals(RuntimeConfig.LogLevel.ERROR.ordinal(), Logger.effectiveLevel());
    }

    @Test
    public void theDefaultReturnsWhenNoSessionIsLive() {
        Logger.registerSession(1, RuntimeConfig.LogLevel.TRACE);
        Logger.unregisterSession(1);

        assertEquals(RuntimeConfig.LogLevel.WARN.ordinal(), Logger.effectiveLevel());
    }

    /**
     * A restart re-registers a live session. Two entries for one id would leave the
     * stale one holding the level open after that session closed.
     */
    @Test
    public void reRegisteringASessionReplacesItsLevel() {
        Logger.registerSession(1, RuntimeConfig.LogLevel.TRACE);
        Logger.registerSession(1, RuntimeConfig.LogLevel.ERROR);

        assertEquals(RuntimeConfig.LogLevel.ERROR.ordinal(), Logger.effectiveLevel());

        Logger.unregisterSession(1);
        assertEquals(RuntimeConfig.LogLevel.WARN.ordinal(), Logger.effectiveLevel());
    }

    /** An embedder who configured nothing gets the default, not the most verbose. */
    @Test
    public void aSessionWithNoConfiguredLevelGetsTheDefault() {
        Logger.registerSession(1, null);

        assertEquals(RuntimeConfig.LogLevel.WARN.ordinal(), Logger.effectiveLevel());
    }
}
