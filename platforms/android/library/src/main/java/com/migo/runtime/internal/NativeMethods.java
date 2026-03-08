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
     * Restart the game session.
     *
     * @param sessionId The session ID
     */
    public static void onRestart(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onRestart(sessionId);
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

    /**
     * Callback for compass data.
     *
     * @param sessionId The session ID
     * @param direction Direction in degrees (0-360, 0 = north)
     * @param accuracy  Accuracy string: "high", "medium", "low", "no-contact", "unreliable", or "unknow X"
     */
    public static void onCompassChange(int sessionId, double direction, String accuracy) {
        if (sessionId >= 0) {
            NativeBridge.onCompassChange(sessionId, direction, accuracy);
        }
    }

    /**
     * Callback for accelerometer data.
     * Called from {@link com.migo.runtime.internal.platform.DeviceSensorManager}.
     *
     * @param sessionId The session ID
     * @param x         Acceleration along X axis in m/s^2
     * @param y         Acceleration along Y axis in m/s^2
     * @param z         Acceleration along Z axis in m/s^2
     */
    public static void onAccelerometerChange(int sessionId, double x, double y, double z) {
        if (sessionId >= 0) {
            NativeBridge.onAccelerometerChange(sessionId, x, y, z);
        }
    }

    // ==================== Network Callbacks ====================

    /**
     * Callback for network status change.
     * Called from {@link com.migo.runtime.internal.platform.NetworkMonitor}.
     *
     * @param sessionId   The session ID
     * @param isConnected Whether network is connected
     * @param networkType Network type: "wifi", "2g", "3g", "4g", "5g", "unknown", "none"
     */
    public static void onNetworkStatusChange(int sessionId, boolean isConnected, String networkType) {
        if (sessionId >= 0) {
            NativeBridge.onNetworkStatusChange(sessionId, isConnected, networkType);
        }
    }

    // ==================== Camera Callbacks ====================

    /**
     * Callback for camera events.
     * Called from {@link com.migo.runtime.internal.platform.CameraManager}.
     *
     * @param sessionId   The session ID
     * @param cameraId    The camera instance ID
     * @param eventType   Event type: "stop", "error", "authCancel", "timeoutCallback"
     * @param jsonPayload JSON-encoded event data
     */
    public static void onCameraEvent(int sessionId, int cameraId, String eventType, String jsonPayload) {
        if (sessionId >= 0 && eventType != null) {
            NativeBridge.onCameraEvent(sessionId, cameraId, eventType,
                    jsonPayload != null ? jsonPayload : "{}");
        }
    }

    /**
     * Callback for camera frame data.
     * Called from {@link com.migo.runtime.internal.platform.CameraManager}.
     *
     * @param sessionId The session ID
     * @param cameraId  The camera instance ID
     * @param frameData Raw frame bytes
     * @param width     Frame width in pixels
     * @param height    Frame height in pixels
     */
    public static void onCameraFrameData(int sessionId, int cameraId, byte[] frameData, int width, int height) {
        if (sessionId >= 0 && frameData != null && width > 0 && height > 0) {
            NativeBridge.onCameraFrameData(sessionId, cameraId, frameData, width, height);
        }
    }

    // ==================== Bluetooth Callbacks ====================

    /**
     * Callback for Bluetooth adapter state change.
     * Called from {@link com.migo.runtime.internal.platform.BluetoothManager}.
     *
     * @param sessionId   The session ID
     * @param available   Whether the adapter is available
     * @param discovering Whether the adapter is discovering devices
     */
    public static void onBluetoothAdapterStateChange(int sessionId, boolean available, boolean discovering) {
        if (sessionId >= 0) {
            NativeBridge.onBluetoothAdapterStateChange(sessionId, available, discovering);
        }
    }

    /**
     * Callback for Bluetooth device found.
     * Called from {@link com.migo.runtime.internal.platform.BluetoothManager}.
     *
     * @param sessionId   The session ID
     * @param devicesJson JSON-encoded array of discovered devices
     */
    public static void onBluetoothDeviceFound(int sessionId, String devicesJson) {
        if (sessionId >= 0 && devicesJson != null) {
            NativeBridge.onBluetoothDeviceFound(sessionId, devicesJson);
        }
    }

    /**
     * Callback for Beacon update.
     * Called from {@link com.migo.runtime.internal.platform.BluetoothManager}.
     *
     * @param sessionId   The session ID
     * @param beaconsJson JSON-encoded array of beacon devices
     */
    public static void onBeaconUpdate(int sessionId, String beaconsJson) {
        if (sessionId >= 0 && beaconsJson != null) {
            NativeBridge.onBeaconUpdate(sessionId, beaconsJson);
        }
    }

    /**
     * Callback for Beacon service state change.
     * Called from {@link com.migo.runtime.internal.platform.BluetoothManager}.
     *
     * @param sessionId   The session ID
     * @param available   Whether the beacon service is available
     * @param discovering Whether the beacon service is discovering
     */
    public static void onBeaconServiceChange(int sessionId, boolean available, boolean discovering) {
        if (sessionId >= 0) {
            NativeBridge.onBeaconServiceChange(sessionId, available, discovering);
        }
    }

    // ==================== Screen Capture ====================

    /**
     * Notify that the user took a screenshot.
     * Triggers migo.onUserCaptureScreen listener in JS.
     *
     * @param sessionId The session ID
     */
    public static void onUserCaptureScreen(int sessionId) {
        if (sessionId >= 0) {
            NativeBridge.onUserCaptureScreen(sessionId);
        }
    }

    // ==================== Keyboard Callbacks ====================

    /**
     * Callback for keyboard input text change.
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static void onKeyboardInput(int sessionId, String value) {
        if (sessionId >= 0 && value != null) {
            NativeBridge.onKeyboardInput(sessionId, value);
        }
    }

    /**
     * Callback for keyboard confirm action.
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static void onKeyboardConfirm(int sessionId, String value) {
        if (sessionId >= 0 && value != null) {
            NativeBridge.onKeyboardConfirm(sessionId, value);
        }
    }

    /**
     * Callback for keyboard complete (dismissed).
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static void onKeyboardComplete(int sessionId, String value) {
        if (sessionId >= 0 && value != null) {
            NativeBridge.onKeyboardComplete(sessionId, value);
        }
    }

    /**
     * Callback for keyboard height change.
     *
     * @param sessionId The session ID
     * @param height    Keyboard height in CSS pixels (0 when hidden)
     */
    public static void onKeyboardHeightChange(int sessionId, double height) {
        if (sessionId >= 0) {
            NativeBridge.onKeyboardHeightChange(sessionId, height);
        }
    }

    // ==================== Recorder Callbacks ====================

    /**
     * Callback for recorder events.
     * Called from {@link com.migo.runtime.internal.platform.AudioRecorderManager}.
     *
     * @param sessionId   The session ID
     * @param eventType   Event type: "start", "pause", "resume", "stop", "error",
     *                    "interruptionBegin", "interruptionEnd"
     * @param jsonPayload JSON-encoded event data
     */
    public static void onRecorderEvent(int sessionId, String eventType, String jsonPayload) {
        if (sessionId >= 0 && eventType != null) {
            NativeBridge.onRecorderEvent(sessionId, eventType, jsonPayload != null ? jsonPayload : "{}");
        }
    }

    /**
     * Callback for recorder frame data.
     * Called from {@link com.migo.runtime.internal.platform.AudioRecorderManager}.
     *
     * @param sessionId   The session ID
     * @param frameData   Raw audio frame bytes
     * @param isLastFrame Whether this is the last frame before stop
     */
    public static void onRecorderFrameData(int sessionId, byte[] frameData, boolean isLastFrame) {
        if (sessionId >= 0 && frameData != null) {
            NativeBridge.onRecorderFrameData(sessionId, frameData, isLastFrame);
        }
    }
}
