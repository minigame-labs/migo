package com.migo.runtime.internal;

import android.view.Surface;

import com.migo.runtime.RuntimeConfig;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * High-level wrapper around native JNI methods.
 * <p>
 * Provides a safer API with proper parameter validation and error handling.
 * This is the recommended way to interact with the native engine.
 *
 * @hide
 */
public final class NativeMethods {

    /**
     * Size of a single TouchPoint structure in bytes.
     * Layout: id(4) + x(4) + y(4) + force(4) + flags(4) = 20 bytes
     */
    private static final int TOUCH_POINT_SIZE = 20;

    /**
     * Maximum number of simultaneous touch points supported.
     */
    private static final int MAX_TOUCH_POINTS = 10;

    /**
     * Thread-local ByteBuffer for touch events to avoid allocation per event.
     */
    private static final ThreadLocal<ByteBuffer> sTouchBuffer = new ThreadLocal<ByteBuffer>() {
        @Override
        protected ByteBuffer initialValue() {
            ByteBuffer buffer = ByteBuffer.allocateDirect(TOUCH_POINT_SIZE * MAX_TOUCH_POINTS);
            buffer.order(ByteOrder.nativeOrder());
            return buffer;
        }
    };

    private NativeMethods() {}

    // ==================== Core Lifecycle ====================

    /**
     * Get the native engine version.
     *
     * @return Version string, or "unknown" if native library failed to load
     */
    public static String getVersion() {
        try {
            String version = NativeBridge.version();
            return version != null ? version : "unknown";
        } catch (UnsatisfiedLinkError e) {
            return "unknown";
        }
    }

    /**
     * Initialize a new session.
     *
     * @param surface Android Surface for rendering
     * @param config  Runtime configuration
     * @return Session ID (>= 0) on success, negative error code on failure
     * @throws IllegalArgumentException if surface or config is null
     */
    public static int init(Surface surface, RuntimeConfig config) {
        if (surface == null) {
            throw new IllegalArgumentException("Surface cannot be null");
        }
        if (config == null) {
            throw new IllegalArgumentException("RuntimeConfig cannot be null");
        }
        return NativeBridge.init(surface, config);
    }

    /**
     * Shut down a session.
     *
     * @param sessionId The session ID to shut down
     */
    public static void shutdown(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.shutdown(sessionId);
        }
    }

    // ==================== Surface Management ====================

    /**
     * Update the rendering surface.
     *
     * @param sessionId The session ID
     * @param surface   New Surface object
     */
    public static void updateSurface(int sessionId, Surface surface) {
        if (sessionId >= 0 && surface != null) {
            NativeBridge.updateSurface(sessionId, surface);
        }
    }

    // ==================== Lifecycle Events ====================

    /**
     * Notify session that it should resume.
     *
     * @param sessionId The session ID
     */
    public static void onShow(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onShow(sessionId);
        }
    }

    /**
     * Notify session that it should pause.
     *
     * @param sessionId The session ID
     */
    public static void onHide(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onHide(sessionId);
        }
    }

    /**
     * Notify session that audio has been interrupted by the system
     * (e.g., incoming phone call).
     *
     * @param sessionId The session ID
     */
    public static void onAudioInterruptionBegin(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onAudioInterruptionBegin(sessionId);
        }
    }

    /**
     * Notify session that audio interruption has ended
     * and playback can resume.
     *
     * @param sessionId The session ID
     */
    public static void onAudioInterruptionEnd(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onAudioInterruptionEnd(sessionId);
        }
    }

    // ==================== Input Events ====================

    /**
     * Send touch events to the session using array parameters.
     *
     * @param sessionId The session ID
     * @param action    Touch action code
     * @param time      Event timestamp in milliseconds
     * @param ids       Array of pointer IDs
     * @param xs        Array of X coordinates
     * @param ys        Array of Y coordinates
     * @param forces    Array of pressure values (0.0-1.0)
     * @param flags     Array of flags (1 = changed pointer)
     */
    public static void onTouch(int sessionId, int action, long time,
                               int[] ids, float[] xs, float[] ys, float[] forces, int[] flags) {
        if (sessionId < 0 || ids == null || ids.length == 0) {
            return;
        }

        int count = Math.min(ids.length, MAX_TOUCH_POINTS);
        ByteBuffer buffer = sTouchBuffer.get();
        buffer.clear();

        for (int i = 0; i < count; i++) {
            buffer.putInt(ids[i]);
            buffer.putFloat(xs != null && i < xs.length ? xs[i] : 0);
            buffer.putFloat(ys != null && i < ys.length ? ys[i] : 0);
            buffer.putFloat(forces != null && i < forces.length ? forces[i] : 1.0f);
            buffer.putInt(flags != null && i < flags.length ? flags[i] : 0);
        }

        buffer.flip();
        NativeBridge.onTouchEvent(sessionId, action, time, count, buffer);
    }

    /**
     * Send raw touch buffer to the session.
     *
     * @param sessionId The session ID
     * @param action    Touch action code
     * @param time      Event timestamp in milliseconds
     * @param count     Number of touch points
     * @param buffer    Pre-packed DirectByteBuffer
     */
    public static void onTouchRaw(int sessionId, int action, long time, int count, ByteBuffer buffer) {
        if (sessionId >= 0 && count > 0 && buffer != null) {
            NativeBridge.onTouchEvent(sessionId, action, time, count, buffer);
        }
    }

    // ==================== Game Loading ====================

    /**
     * Load and run a game module.
     * <p>
     * Game paths are derived from gameId and base directories (filesDir, cacheDir)
     * configured in RuntimeConfig.
     *
     * @param sessionId The session ID
     * @param gameId    Unique game identifier (1-64 alphanumeric, underscore, hyphen)
     * @param entry     Entry point file (e.g., "game.js")
     * @return 0 on success, negative error code on failure
     */
    public static int modMain(int sessionId, String gameId, String entry) {
        if (sessionId < 0 || gameId == null || entry == null) {
            return -1;
        }
        return NativeBridge.modMain(sessionId, gameId, entry);
    }

    // ==================== System Callbacks ====================

    /**
     * Callback for Bluetooth setting result.
     *
     * @param sessionId The session ID
     * @param enabled   Whether Bluetooth is now enabled
     */
    public static void onBluetoothSettingResult(int sessionId, boolean enabled) {
        if (sessionId >= 0) {
            NativeBridge.onOpenSystemBluetoothSetting(sessionId, enabled ? 1 : 0);
        }
    }

    /**
     * Callback for app authorization setting result.
     *
     * @param sessionId The session ID
     * @param code      0 on success, negative on failure
     */
    public static void onAppAuthorizeSettingResult(int sessionId, int code) {
        if (sessionId >= 0) {
            NativeBridge.onOpenAppAuthorizeSetting(sessionId, code);
        }
    }

    /**
     * Callback for modal dialog result.
     *
     * @param sessionId The session ID
     * @param confirm   1 if user tapped confirm, 0 otherwise
     * @param cancel    1 if user tapped cancel, 0 otherwise
     */
    public static void onModalResult(int sessionId, int confirm, int cancel) {
        if (sessionId >= 0) {
            NativeBridge.onModalResult(sessionId, confirm, cancel);
        }
    }

    /**
     * Callback for action sheet result.
     *
     * @param sessionId The session ID
     * @param tapIndex  Index of selected item (0-based), or -1 if cancelled
     */
    public static void onActionSheetResult(int sessionId, int tapIndex) {
        if (sessionId >= 0) {
            NativeBridge.onActionSheetResult(sessionId, tapIndex);
        }
    }

    // ==================== Device Sensor Callbacks ====================

    /**
     * Callback for device motion sensor data.
     * Called from {@link com.migo.runtime.internal.platform.DeviceSensorManager}.
     *
     * @param sessionId The session ID
     * @param alpha     Rotation around Z axis in degrees (0-360)
     * @param beta      Rotation around X axis in degrees (-180 to 180)
     * @param gamma     Rotation around Y axis in degrees (-90 to 90)
     */
    public static void onDeviceMotionChange(int sessionId, double alpha, double beta, double gamma) {
        if (sessionId >= 0) {
            NativeBridge.onDeviceMotionChange(sessionId, alpha, beta, gamma);
        }
    }

    /**
     * Callback for gyroscope sensor data.
     * Called from {@link com.migo.runtime.internal.platform.DeviceSensorManager}.
     *
     * @param sessionId The session ID
     * @param x         Angular velocity around X axis in rad/s
     * @param y         Angular velocity around Y axis in rad/s
     * @param z         Angular velocity around Z axis in rad/s
     */
    public static void onGyroscopeChange(int sessionId, double x, double y, double z) {
        if (sessionId >= 0) {
            NativeBridge.onGyroscopeChange(sessionId, x, y, z);
        }
    }

    /**
     * Callback for device orientation change.
     *
     * @param sessionId The session ID
     * @param value     One of: "portrait", "landscape", "landscapeReverse"
     */
    public static void onDeviceOrientationChange(int sessionId, String value) {
        if (sessionId >= 0 && value != null) {
            NativeBridge.onDeviceOrientationChange(sessionId, value);
        }
    }
}
