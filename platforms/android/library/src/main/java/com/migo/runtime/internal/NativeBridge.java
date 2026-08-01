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
     * Get the minimum Android API level the native engine was
     * compiled for.
     * <p>
     * Sourced from the build script (see
     * {@code scripts/build-android-so.sh::ANDROID_API}) so the Java
     * SDK cannot silently drift from the native ABI floor.  The
     * value changes only when the Rust engine is rebuilt against a
     * different NDK preset.
     *
     * @return The minimum API level (e.g., 26)
     */
    public static native int getMinApiLevel();

    /**
     * Load an external {@code icudtl.dat} file so SkParagraph can
     * perform text shaping.  The host should copy the bundled
     * asset to the app's private cache dir once per install and
     * pass the resulting absolute path here before creating any
     * {@code GameSession}.
     * <p>
     * When the native engine was built with the embedded ICU
     * payload (the default), this method returns {@code true}
     * immediately without touching the file.  When built with the
     * {@code external_icudtl} Cargo feature, the native side calls
     * Skia's {@code SkLoadICU(path)} and returns the success code.
     * <p>
     * Callers that always ship the embedded path can skip this
     * method entirely; callers that strip the ICU payload to keep
     * {@code libmigo.so} small MUST call it before the first text
     * layout op.
     *
     * @param icuDataPath Absolute path to the {@code icudtl.dat}
     *                    file on device.
     * @return {@code true} if ICU data is ready (either embedded
     *         or successfully loaded), {@code false} on failure.
     */
    public static native boolean initIcuData(String icuDataPath);

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
     * @param width     Surface buffer width in physical pixels
     * @param height    Surface buffer height in physical pixels
     * @param density   Current device-pixel ratio for this display
     */
    public static native void updateSurface(
            int sessionId, Object surface, int width, int height, float density);

    /**
     * Notify that the current rendering surface has been destroyed.
     *
     * @param sessionId The session ID
     */
    public static native void onSurfaceDestroyed(int sessionId);

    // ==================== Lifecycle Events ====================

    /**
     * Notify that the session should show (resume rendering).
     *
     * @param sessionId   The session ID
     * @param optionsJson Launch/enter options JSON (scene/query/referrerInfo/shareTicket)
     */
    public static native void onShow(int sessionId, String optionsJson);

    /**
     * Notify that the session should hide (pause rendering).
     *
     * @param sessionId The session ID
     */
    public static native void onHide(int sessionId);

    /**
     * Restart the game session.
     *
     * @param sessionId The session ID
     */
    public static native void onRestart(int sessionId);

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

    /**
     * Callback when compass data is available.
     *
     * @param sessionId The session ID
     * @param direction Direction in degrees (0-360, 0 = north)
     * @param accuracy  Accuracy string: "high", "medium", "low", "no-contact", "unreliable"
     */
    public static native void onCompassChange(int sessionId, double direction, String accuracy);

    /**
     * Callback when accelerometer data is available.
     *
     * @param sessionId The session ID
     * @param x         Acceleration along X axis in m/s^2
     * @param y         Acceleration along Y axis in m/s^2
     * @param z         Acceleration along Z axis in m/s^2
     */
    public static native void onAccelerometerChange(int sessionId, double x, double y, double z);

    // ==================== Network Callbacks ====================

    /**
     * Callback when network status changes.
     *
     * @param sessionId   The session ID
     * @param isConnected Whether network is connected
     * @param networkType Network type: "wifi", "2g", "3g", "4g", "5g", "unknown", "none"
     */
    public static native void onNetworkStatusChange(int sessionId, boolean isConnected, String networkType);

    // ==================== VSync (Choreographer) ====================

    /**
     * Send a VSync signal to the render thread.
     * Called from {@link com.migo.runtime.internal.VsyncScheduler} on each Choreographer frame callback.
     *
     * @param sessionId      The session ID
     * @param frameTimeNanos Frame timestamp from Choreographer in nanoseconds
     */
    public static native void onVsync(int sessionId, long frameTimeNanos);

    /**
     * Notify the native render thread of the display refresh period.
     * Called once at session start and when the display refresh rate changes.
     *
     * @param sessionId          The session ID
     * @param refreshPeriodNanos Display refresh period in nanoseconds
     *                           (e.g., 16666667 for 60Hz, 8333333 for 120Hz)
     */
    public static native void setDisplayRefreshRate(int sessionId, long refreshPeriodNanos);

    // ==================== Recorder Callbacks ====================

    /**
     * Callback when a recorder event occurs (start, pause, resume, stop, error, interruption).
     *
     * @param sessionId   The session ID
     * @param eventType   Event type string
     * @param jsonPayload JSON-encoded event data
     */
    public static native void onRecorderEvent(int sessionId, String eventType, String jsonPayload);

    /**
     * Callback when recorder frame data is available.
     *
     * @param sessionId   The session ID
     * @param frameData   Raw audio frame bytes
     * @param isLastFrame Whether this is the last frame before stop
     */
    public static native void onRecorderFrameData(int sessionId, byte[] frameData, boolean isLastFrame);

    // ==================== Camera Callbacks ====================

    /**
     * Callback when a camera event occurs (stop, error, authCancel, timeoutCallback).
     *
     * @param sessionId   The session ID
     * @param cameraId    The camera instance ID
     * @param eventType   Event type string
     * @param jsonPayload JSON-encoded event data
     */
    public static native void onCameraEvent(int sessionId, int cameraId, String eventType, String jsonPayload);

    /**
     * Callback when camera frame data is available.
     *
     * <p>The three YUV_420_888 plane buffers are passed by reference (direct
     * ByteBuffers) together with each plane's window; native code copies the
     * {@code [offset, offset + length)} bytes synchronously. The buffers must
     * stay valid (their Image open) for the duration of this call.
     *
     * @param sessionId The session ID
     * @param cameraId  The camera instance ID
     * @param yBuffer   Direct Y-plane ByteBuffer
     * @param yOffset   Y-plane window start (buffer position)
     * @param yLength   Y-plane window length (buffer remaining)
     * @param uBuffer   Direct U-plane ByteBuffer
     * @param uOffset   U-plane window start
     * @param uLength   U-plane window length
     * @param vBuffer   Direct V-plane ByteBuffer
     * @param vOffset   V-plane window start
     * @param vLength   V-plane window length
     * @param width     Frame width in pixels
     * @param height    Frame height in pixels
     */
    public static native void onCameraFrameData(int sessionId, int cameraId,
            ByteBuffer yBuffer, int yOffset, int yLength,
            ByteBuffer uBuffer, int uOffset, int uLength,
            ByteBuffer vBuffer, int vOffset, int vLength,
            int width, int height);

    // ==================== Bluetooth Callbacks ====================

    /**
     * Callback when Bluetooth adapter state changes.
     *
     * @param sessionId   The session ID
     * @param available   Whether the adapter is available
     * @param discovering Whether the adapter is discovering devices
     */
    public static native void onBluetoothAdapterStateChange(int sessionId, boolean available, boolean discovering);

    /**
     * Callback when Bluetooth devices are found during discovery.
     *
     * @param sessionId   The session ID
     * @param devicesJson JSON-encoded array of discovered devices
     */
    public static native void onBluetoothDeviceFound(int sessionId, String devicesJson);

    /**
     * Callback when Beacon devices are updated during discovery.
     *
     * @param sessionId   The session ID
     * @param beaconsJson JSON-encoded array of beacon devices
     */
    public static native void onBeaconUpdate(int sessionId, String beaconsJson);

    /**
     * Callback when Beacon service state changes.
     *
     * @param sessionId   The session ID
     * @param available   Whether the beacon service is available
     * @param discovering Whether the beacon service is discovering
     */
    public static native void onBeaconServiceChange(int sessionId, boolean available, boolean discovering);

    // ==================== BLE GATT Callbacks ====================

    public static native void onBLEConnectionStateChange(int sessionId, String deviceId, boolean connected);

    public static native void onBLECharacteristicValueChange(int sessionId, String deviceId,
                                                              String serviceId, String characteristicId, byte[] value);

    public static native void onBLEMTUChange(int sessionId, String deviceId, int mtu);

    // ==================== Keyboard Callbacks ====================

    /**
     * Callback when soft keyboard input text changes.
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static native void onKeyboardInput(int sessionId, String value);

    /**
     * Callback when user presses confirm on soft keyboard.
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static native void onKeyboardConfirm(int sessionId, String value);

    /**
     * Callback when soft keyboard is dismissed/completed.
     *
     * @param sessionId The session ID
     * @param value     Current text value
     */
    public static native void onKeyboardComplete(int sessionId, String value);

    /**
     * Callback when soft keyboard height changes.
     *
     * @param sessionId The session ID
     * @param height    Keyboard height in CSS pixels (0 when hidden)
     */
    public static native void onKeyboardHeightChange(int sessionId, double height);

    // ==================== Screen Capture ====================

    /**
     * Notify that the user took a screenshot (system screenshot button).
     * Triggers migo.onUserCaptureScreen listener in JS.
     *
     * @param sessionId The session ID
     */
    public static native void onUserCaptureScreen(int sessionId);

    // ==================== Memory Warning ====================

    /**
     * Notify that the system has sent a memory warning.
     * Triggers migo.onMemoryWarning listener in JS.
     *
     * @param sessionId The session ID
     * @param level     Memory warning level (Android TRIM_MEMORY_* constant):
     *                  5 = RUNNING_MODERATE, 10 = RUNNING_LOW, 15 = RUNNING_CRITICAL
     */
    public static native void onMemoryWarning(int sessionId, int level);

    // ==================== ADPF Thermal ====================

    /**
     * Notify that the device thermal status has changed (ADPF, API 29+).
     *
     * @param sessionId The session ID
     * @param status    Thermal status level: 0=none, 1=light, 2=moderate,
     *                  3=severe, 4=critical, 5=emergency, 6=shutdown
     */
    public static native void onThermalStatusChanged(int sessionId, int status);

    // ==================== Image API Callbacks ====================

    /**
     * Callback when compressImage operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with tempFilePath or error
     */
    public static native void onCompressImageResult(int sessionId, String resultJson);

    /**
     * Callback when chooseImage operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with tempFilePaths/tempFiles or error
     */
    public static native void onChooseImageResult(int sessionId, String resultJson);

    /**
     * Callback when chooseMessageFile operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with tempFiles or error
     */
    public static native void onChooseMessageFileResult(int sessionId, String resultJson);

    // ==================== Location Callbacks ====================

    /**
     * Callback when getLocation operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with location data or error
     */
    public static native void onLocationResult(int sessionId, String resultJson);

    /**
     * Callback when getFuzzyLocation operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with location data or error
     */
    public static native void onFuzzyLocationResult(int sessionId, String resultJson);

    // ==================== Scan Code Callbacks ====================

    /**
     * Callback when scanCode operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON-encoded result with scan data or error
     */
    public static native void onScanCodeResult(int sessionId, String resultJson);

    // ==================== Auth Callbacks ====================

    /**
     * Callback when login operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId/code or error
     */
    public static native void onLoginResult(int sessionId, String resultJson);

    /**
     * Deliver one ad event (load / error / close / resize / hide) to content.
     *
     * @param sessionId The session ID
     * @param eventJson JSON carrying adId, event name and event fields
     */
    public static native void onAdEvent(int sessionId, String eventJson);

    /**
     * Callback when checkSession operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId or error
     */
    public static native void onCheckSessionResult(int sessionId, String resultJson);

    /**
     * Callback when getUserInfo operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId/userInfo or error
     */
    public static native void onGetUserInfoResult(int sessionId, String resultJson);

    /**
     * Callback when getPhoneNumber operation completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId/code or error
     */
    public static native void onGetPhoneNumberResult(int sessionId, String resultJson);

    // ==================== Subpackage Callbacks ====================

    /**
     * Callback for subpackage download progress.
     * Called from the host app's download manager.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId, progress, totalBytesWritten, totalBytesExpectedToWrite
     */
    public static native void onSubpackageProgress(int sessionId, String resultJson);

    /**
     * Callback when subpackage download completes (success or failure).
     * Called from the host app's download manager.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with requestId (and optionally "error" on failure)
     */
    public static native void onSubpackageResult(int sessionId, String resultJson);

    // ==================== Debug Stats ====================

    /**
     * Get debug statistics from the render thread.
     * Returns a 20-byte array containing (all little-endian u32):
     * <ul>
     *   <li>[0..4) fps_x10 — FPS multiplied by 10 (e.g., 598 = 59.8 FPS)</li>
     *   <li>[4..8) frame_time_us — Last frame time in microseconds</li>
     *   <li>[8..12) dropped_frames — Total dropped RAF signals</li>
     *   <li>[12..16) fatal_error_code — Last fatal engine error code (0 = none)</li>
     *   <li>[16..20) first_frame_ms — Milliseconds from render thread start to first frame (0 = not yet)</li>
     * </ul>
     *
     * @param sessionId The session ID
     * @return byte array with stats, or null if session not found
     */
    public static native byte[] getDebugStats(int sessionId);

    // ==================== Setting Callback ====================

    /**
     * Callback with setting result after openSetting completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with authSetting, or error
     */
    public static native void onOpenSettingResult(int sessionId, String resultJson);

    // ==================== Share Callback ====================

    /**
     * Callback with share result after shareAppMessage completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with result, or error
     */
    public static native void onShareAppMessageResult(int sessionId, String resultJson);

    // ==================== Navigate Callback ====================

    /**
     * Callback with navigation result after navigateToMiniProgram completes.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with result, or error
     */
    public static native void onNavigateToMiniProgramResult(int sessionId, String resultJson);

    // ==================== Payment Callbacks ====================

    /**
     * Callback with Midas payment result.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with result (include requestId), or error
     */
    public static native void onMidasPaymentResult(int sessionId, String resultJson);

    /**
     * Callback with Midas payment game item result.
     *
     * @param sessionId  The session ID
     * @param resultJson JSON with result (include requestId), or error
     */
    public static native void onMidasPaymentGameItemResult(int sessionId, String resultJson);

    // ==================== Video Callbacks ====================

    /**
     * Callback when a video player event occurs (play, pause, ended, error, timeupdate, etc.).
     *
     * @param sessionId The session ID
     * @param videoId   The video player instance ID
     * @param eventType Event type string
     * @param dataJson  JSON-encoded event data
     */
    public static native void onVideoEvent(int sessionId, int videoId, String eventType, String dataJson);

    // ==================== Script Execution ====================

    /**
     * Execute a JavaScript snippet in the game's V8 runtime.
     * <p>
     * The script is evaluated asynchronously on the host thread.
     *
     * @param sessionId The session ID
     * @param script    JavaScript source code to evaluate
     * @return 0 on success, -1 on failure
     */
    public static native int executeScript(int sessionId, String script);

    // ==================== Console Logs ====================

    /**
     * Get console log entries written since the given cursor.
     * <p>
     * Returns a JSON string:
     * {@code {"logs":[{"l":level,"t":timestamp,"m":"message"},...], "cursor": N}}
     * <p>
     * Pass cursor=0 on the first call, then use the returned cursor value
     * for subsequent calls to get only new entries.
     *
     * @param sessionId   The session ID
     * @param sinceCursor Cursor from the previous call (0 for first call)
     * @return JSON string with log entries, or null if session not found
     */
    public static native String getConsoleLogs(int sessionId, long sinceCursor);

    // ==================== AHardwareBuffer helpers ====================

    /**
     * Extract the native {@code AHardwareBuffer*} backing a Java
     * {@link android.hardware.HardwareBuffer} and return it as a
     * {@code long} (pointer value). Used by the zero-copy image
     * decode path so Rust can import the buffer via
     * {@code eglCreateImageKHR(EGL_NATIVE_BUFFER_ANDROID, …)} without
     * shuttling pixel bytes through JNI.
     * <p>
     * Returns 0 if the handle is null or the underlying AHB can't be
     * obtained. On success the native side has already retained one
     * extra {@code AHardwareBuffer} refcount, so callers may close
     * the Java {@link android.hardware.HardwareBuffer} wrapper after
     * this method returns and transfer ownership of that native ref
     * to Rust.
     */
    public static native long nativeAhbPointerFromHardwareBuffer(
            android.hardware.HardwareBuffer hb);
}
