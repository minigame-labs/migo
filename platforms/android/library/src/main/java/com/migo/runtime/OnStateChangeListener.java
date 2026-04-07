package com.migo.runtime;

/**
 * Listener for {@link GameSession} lifecycle state changes.
 * Callbacks are always delivered on the main thread.
 */
public interface OnStateChangeListener {
    /**
     * Called when the session transitions to a new state.
     *
     * @param session the session whose state changed
     * @param oldState the previous state
     * @param newState the current state
     */
    void onStateChanged(GameSession session, SessionState oldState, SessionState newState);
}
