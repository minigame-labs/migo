package com.migo.runtime.internal.platform;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/** Host-JVM tests for the platform lifecycle request state machine. */
public final class LifecycleRequestStateTest {
    /**
     * The destroyed flag is observed in both states.
     *
     * It gates every transition, so a reader stuck at one answer either refuses a live
     * session's requests or admits a destroyed one's. Only one polarity was ever asserted.
     */
    @Test
    public void theDestroyedFlagIsObservedBeforeAndAfterDestruction() {
        LifecycleRequestState<Boolean> state = new LifecycleRequestState<>(false);

        assertFalse("a fresh state is not destroyed", state.isDestroyed());
        state.destroy();
        assertTrue("a destroyed state says so", state.isDestroyed());
    }

    @Test
    public void startAndRestartUseLatestRequest() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);

        assertEquals(LifecycleRequestState.Action.START, state.requestStart("game"));
        assertTrue(state.isActive());
        assertEquals("game", state.getRequest());

        assertEquals(LifecycleRequestState.Action.RESTART, state.requestStart("ui"));
        assertTrue(state.isActive());
        assertEquals("ui", state.getRequest());
    }

    @Test
    public void suspendRetainsRequestAndResumesOnce() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);
        state.requestStart("normal");

        assertEquals(LifecycleRequestState.Action.STOP, state.suspend());
        assertTrue(state.isSuspended());
        assertTrue(state.isRequested());
        assertFalse(state.isActive());
        assertEquals("normal", state.getRequest());
        assertEquals(LifecycleRequestState.Action.NONE, state.suspend());

        assertEquals(LifecycleRequestState.Action.START, state.resume());
        assertFalse(state.isSuspended());
        assertTrue(state.isActive());
        assertEquals(LifecycleRequestState.Action.NONE, state.resume());
    }

    @Test
    public void hiddenStopPreventsResume() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);
        state.requestStart("game");
        state.suspend();

        assertEquals(LifecycleRequestState.Action.NONE, state.requestStop());
        assertFalse(state.isRequested());
        assertNull(state.getRequest());
        assertEquals(LifecycleRequestState.Action.NONE, state.resume());
        assertFalse(state.isActive());
    }

    @Test
    public void hiddenStartDefersLatestRequest() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(true);

        assertEquals(LifecycleRequestState.Action.NONE, state.requestStart("normal"));
        assertEquals(LifecycleRequestState.Action.NONE, state.requestStart("game"));
        assertFalse(state.isActive());
        assertEquals("game", state.getRequest());
        assertEquals(LifecycleRequestState.Action.START, state.resume());
        assertTrue(state.isActive());
    }

    @Test
    public void activationFailureCanRetainRequest() {
        LifecycleRequestState<String> retry = new LifecycleRequestState<>(false);
        retry.requestStart("game");
        retry.startFailed(true);
        assertTrue(retry.isRequested());
        assertFalse(retry.isActive());
        assertEquals(LifecycleRequestState.Action.NONE, retry.suspend());
        assertEquals(LifecycleRequestState.Action.START, retry.resume());
    }

    @Test
    public void activationFailureCanCancelRequest() {
        LifecycleRequestState<String> cancel = new LifecycleRequestState<>(false);
        cancel.requestStart("game");
        cancel.startFailed(false);
        assertFalse(cancel.isRequested());
        assertFalse(cancel.isActive());
        assertNull(cancel.getRequest());
    }

    @Test
    public void destroyIsTerminalAndIdempotent() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);
        state.requestStart("game");

        assertEquals(LifecycleRequestState.Action.STOP, state.destroy());
        assertTrue(state.isDestroyed());
        assertFalse(state.isRequested());
        assertFalse(state.isActive());
        assertEquals(LifecycleRequestState.Action.NONE, state.destroy());
        assertEquals(LifecycleRequestState.Action.NONE, state.requestStart("ui"));
        assertEquals(LifecycleRequestState.Action.NONE, state.resume());
    }

    @Test
    public void releasedResourceRejectsQueuedCommands() {
        ResourceLifetime lifetime = new ResourceLifetime();
        int[] executions = {0};
        Runnable queuedCommand = () -> {
            if (lifetime.canRun()) {
                executions[0]++;
            }
        };

        assertTrue(lifetime.canRun());
        assertTrue(lifetime.release());
        queuedCommand.run();

        assertEquals(0, executions[0]);
        assertFalse(lifetime.canRun());
        assertFalse(lifetime.release());
    }
}
