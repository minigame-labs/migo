package com.migo.runtime.callback;

/**
 * Listener for game lifecycle events.
 * <p>
 * Implement this interface to receive notifications about game state changes.
 *
 * <pre>{@code
 * session.setOnGameEventListener(new OnGameEventListener() {
 *     @Override
 *     public void onGameReady() {
 *         Log.i(TAG, "Game is ready to play");
 *     }
 *
 *     @Override
 *     public void onGameExit(int exitCode) {
 *         Log.i(TAG, "Game exited with code: " + exitCode);
 *     }
 * });
 * }</pre>
 *
 * @since 1.0.0
 */
public interface OnGameEventListener {

    /**
     * Called when the game has finished loading and is ready to run.
     * <p>
     * This is called on the main thread.
     */
    void onGameReady();

    /**
     * Called when the game has exited.
     * <p>
     * This is called on the main thread.
     *
     * @param exitCode The exit code (0 for normal exit, non-zero for errors)
     */
    void onGameExit(int exitCode);

    /**
     * Called when the game requests to show a loading screen.
     * <p>
     * The host app should display a loading indicator.
     * Default implementation does nothing.
     */
    default void onLoadingStart() {}

    /**
     * Called when loading is complete and the loading screen can be hidden.
     * <p>
     * Default implementation does nothing.
     */
    default void onLoadingEnd() {}

    /**
     * Called periodically during loading with progress information.
     * <p>
     * Default implementation does nothing.
     *
     * @param progress Progress value from 0.0 to 1.0
     * @param message  Optional message describing current loading step
     */
    default void onLoadingProgress(float progress, String message) {}
}
