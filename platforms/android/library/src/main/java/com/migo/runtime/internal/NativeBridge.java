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

    /**
     * Notify that audio playback has been interrupted by the system.
     *
     * @param sessionId The session ID
     */
    public static native void onAudioInterruptionBegin(int sessionId);

    /**
     * Notify that audio interruption has ended.
     *
     * @param sessionId The session ID
     */
    public static native void onAudioInterruptionEnd(int sessionId);

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

    /**
     * Callback when app authorization setting result is received.
     *
     * @param sessionId The session ID
     * @param code      0 on success, negative on failure
     */
    public static native void onOpenAppAuthorizeSetting(int sessionId, int code);

    /**
     * Callback when modal dialog is dismissed.
     *
     * @param sessionId The session ID
     * @param confirm   1 if user tapped confirm, 0 otherwise
     * @param cancel    1 if user tapped cancel, 0 otherwise
     */
    public static native void onModalResult(int sessionId, int confirm, int cancel);

    /**
     * Callback when action sheet is dismissed.
     *
     * @param sessionId The session ID
     * @param tapIndex  Index of selected item (0-based), or -1 if cancelled
     */
    public static native void onActionSheetResult(int sessionId, int tapIndex);

    // ==================== Device Sensor Callbacks ====================

    /**
     * Callback when device motion (rotation vector) data is available.
     *
     * @param sessionId The session ID
     * @param alpha     Rotation around Z axis in degrees (0-360)
     * @param beta      Rotation around X axis in degrees (-180 to 180)
     * @param gamma     Rotation around Y axis in degrees (-90 to 90)
     */
    public static native void onDeviceMotionChange(int sessionId, double alpha, double beta, double gamma);

    /**
     * Callback when gyroscope data is available.
     *
     * @param sessionId The session ID
     * @param x         Angular velocity around X axis in rad/s
     * @param y         Angular velocity around Y axis in rad/s
     * @param z         Angular velocity around Z axis in rad/s
     */
    public static native void onGyroscopeChange(int sessionId, double x, double y, double z);

    /**
     * Callback when device screen orientation changes.
     *
     * @param sessionId The session ID
     * @param value     One of: "portrait", "landscape", "landscapeReverse"
     */
    public static native void onDeviceOrientationChange(int sessionId, String value);
}
