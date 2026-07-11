package com.migo.runtime.internal.platform;

public final class LifecycleRequestStateTestMain {
    public static void main(String[] args) {
        startAndRestartUseLatestRequest();
        suspendRetainsRequestAndResumesOnce();
        hiddenStopPreventsResume();
        hiddenStartDefersLatestRequest();
        activationFailureCanRetainOrCancelRequest();
        destroyIsTerminalAndIdempotent();
        releasedResourceRejectsQueuedCommands();
        System.out.println("LifecycleRequestState tests passed");
    }

    private static void startAndRestartUseLatestRequest() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);

        assertEquals(LifecycleRequestState.Action.START, state.requestStart("game"));
        assertTrue(state.isActive());
        assertEquals("game", state.getRequest());

        assertEquals(LifecycleRequestState.Action.RESTART, state.requestStart("ui"));
        assertTrue(state.isActive());
        assertEquals("ui", state.getRequest());
    }

    private static void suspendRetainsRequestAndResumesOnce() {
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

    private static void hiddenStopPreventsResume() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(false);
        state.requestStart("game");
        state.suspend();

        assertEquals(LifecycleRequestState.Action.NONE, state.requestStop());
        assertFalse(state.isRequested());
        assertEquals(null, state.getRequest());
        assertEquals(LifecycleRequestState.Action.NONE, state.resume());
        assertFalse(state.isActive());
    }

    private static void hiddenStartDefersLatestRequest() {
        LifecycleRequestState<String> state = new LifecycleRequestState<>(true);

        assertEquals(LifecycleRequestState.Action.NONE, state.requestStart("normal"));
        assertEquals(LifecycleRequestState.Action.NONE, state.requestStart("game"));
        assertFalse(state.isActive());
        assertEquals("game", state.getRequest());
        assertEquals(LifecycleRequestState.Action.START, state.resume());
        assertTrue(state.isActive());
    }

    private static void activationFailureCanRetainOrCancelRequest() {
        LifecycleRequestState<String> retry = new LifecycleRequestState<>(false);
        retry.requestStart("game");
        retry.startFailed(true);
        assertTrue(retry.isRequested());
        assertFalse(retry.isActive());
        assertEquals(LifecycleRequestState.Action.NONE, retry.suspend());
        assertEquals(LifecycleRequestState.Action.START, retry.resume());

        LifecycleRequestState<String> cancel = new LifecycleRequestState<>(false);
        cancel.requestStart("game");
        cancel.startFailed(false);
        assertFalse(cancel.isRequested());
        assertFalse(cancel.isActive());
        assertEquals(null, cancel.getRequest());
    }

    private static void destroyIsTerminalAndIdempotent() {
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

    private static void releasedResourceRejectsQueuedCommands() {
        ResourceLifetime lifetime = new ResourceLifetime();
        int[] executions = {0};
        Runnable queuedCommand = () -> {
            if (lifetime.canRun()) executions[0]++;
        };

        assertTrue(lifetime.canRun());
        assertTrue(lifetime.release());
        queuedCommand.run();

        assertEquals(0, executions[0]);
        assertFalse(lifetime.canRun());
        assertFalse(lifetime.release());
    }

    private static void assertTrue(boolean value) {
        if (!value) throw new AssertionError("expected true");
    }

    private static void assertFalse(boolean value) {
        if (value) throw new AssertionError("expected false");
    }

    private static void assertEquals(Object expected, Object actual) {
        if (expected == null ? actual != null : !expected.equals(actual)) {
            throw new AssertionError("expected=" + expected + ", actual=" + actual);
        }
    }

    private LifecycleRequestStateTestMain() {}
}
