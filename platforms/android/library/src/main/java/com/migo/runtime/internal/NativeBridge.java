package com.migo.runtime.internal;

import com.migo.runtime.RuntimeConfig;

import java.nio.ByteBuffer;

/**
 * JNI bridge class containing native method declarations.
 * <p>
 * These methods are implemented in Rust and registered via JNI_OnLoad.
 * Do not call these methods directly - use {@link NativeMethods} instead.
 *
 * @hide
 */
public final class NativeBridge {

    // Native library is loaded by MigoRuntime

    private NativeBridge() {}

    // ==================== Core Lifecycle ====================

    /**
     * Get the native engine version string.
     *
     * @return Version string (e.g., "0.1.0")
     */
    public static native String version();

    /**
     * Initialize a new session with the given surface and options.
     *
     * @param surface Android Surface object for rendering
     * @param config  Runtime configuration
     * @return Session ID (>= 0) on success, negative error code on failure
     */
    public static native int init(Object surface, RuntimeConfig config);

    /**
     * Shut down a session and release all resources.
     *
     * @param sessionId The session ID returned by init()
     */
    public static native void shutdown(int sessionId);

    // ==================== Surface Management ====================

    /**
     * Notify that a surface has been updated/recreated.
     *
     * @param sessionId The session ID
     * @param surface   New Android Surface object
     */
    public static native void updateSurface(int sessionId, Object surface);

    // ==================== Lifecycle Events ====================

    /**
     * Notify that the session should show (resume rendering).
     *
     * @param sessionId The session ID
     */
    public static native void onShow(int sessionId);

    /**
     * Notify that the session should hide (pause rendering).
     *
     * @param sessionId The session ID
     */
    public static native void onHide(int sessionId);

    // ==================== Input Events ====================

    /**
     * Send touch events to the session.
     *
     * @param sessionId The session ID
     * @param action    Touch action (0=DOWN, 1=UP, 2=MOVE, 3=CANCEL, 5=POINTER_DOWN, 6=POINTER_UP)
     * @param time      Event timestamp in milliseconds
     * @param count     Number of touch points
     * @param buffer    DirectByteBuffer containing packed TouchPoint data
     */
    public static native void onTouchEvent(int sessionId, int action, long time, int count, ByteBuffer buffer);

    // ==================== Game Loading ====================

    /**
     * Load and run a game module.
     * <p>
     * Game paths are derived from gameId and base directories (filesDir, cacheDir)
     * configured in RuntimeConfig. The native layer creates isolated directories:
     * <ul>
     *   <li>/code - Game code (read-only)</li>
     *   <li>/user - User data/saves (read-write)</li>
     *   <li>/cache - Cache files (read-write)</li>
     *   <li>/tmp - Temporary files (read-write)</li>
     * </ul>
     *
     * @param sessionId The session ID
     * @param gameId    Unique game identifier (1-64 alphanumeric, underscore, hyphen)
     * @param entry     Entry point file name (e.g., "game.js")
     * @return 0 on success, negative error code on failure
     */
    public static native int modMain(int sessionId, String gameId, String entry);

    // ==================== System Callbacks ====================

    /**
     * Callback when system Bluetooth setting result is received.
     *
     * @param sessionId The session ID
     * @param enabled   1 if Bluetooth was enabled, 0 otherwise
     */
    public static native void onOpenSystemBluetoothSetting(int sessionId, int enabled);
}
