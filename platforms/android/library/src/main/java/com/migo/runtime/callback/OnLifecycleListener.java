package com.migo.runtime.callback;

/**
 * Listener for runtime lifecycle events.
 * <p>
 * Implement this interface to receive notifications about runtime state changes.
 *
 * @since 1.0.0
 */
public interface OnLifecycleListener {

    /**
     * Called when the runtime has been initialized.
     */
    void onInitialized();

    /**
     * Called when the runtime is being destroyed.
     */
    void onDestroyed();

    /**
     * Called when the runtime is paused (e.g., app goes to background).
     */
    default void onPaused() {}

    /**
     * Called when the runtime is resumed (e.g., app comes to foreground).
     */
    default void onResumed() {}

    /**
     * Called when the rendering surface is created or changed.
     *
     * @param width  New surface width in pixels
     * @param height New surface height in pixels
     */
    default void onSurfaceChanged(int width, int height) {}
}
