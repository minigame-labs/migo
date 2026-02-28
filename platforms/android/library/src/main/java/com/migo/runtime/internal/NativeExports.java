package com.migo.runtime.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.provider.Settings;

import com.migo.runtime.internal.platform.BatteryInfo;
import com.migo.runtime.internal.platform.Clipboard;
import com.migo.runtime.internal.platform.DeviceInfo;
import com.migo.runtime.internal.platform.DeviceSensorManager;
import com.migo.runtime.internal.platform.InteractionUI;
import com.migo.runtime.internal.platform.DisplayCompat;
import com.migo.runtime.internal.platform.NetworkMonitor;
import com.migo.runtime.internal.platform.Permissions;
import com.migo.runtime.internal.platform.ScreenBrightness;
import com.migo.runtime.internal.platform.SystemSettings;
import com.migo.runtime.internal.platform.Vibrator;
import com.migo.runtime.internal.platform.AudioRecorderManager;
import com.migo.runtime.internal.platform.CameraManager;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.os.Handler;
import android.os.Looper;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Static methods exposed to native code via JNI.
 * <p>
 * These methods are called from Rust/native code to access Android platform features.
 * Method signatures must match those registered in registration.rs.
 *
 * @hide
 */
public final class NativeExports {

    private static final int BLUETOOTH_SETTING_REQUEST_CODE = 10001;
    private static final int APP_AUTHORIZE_SETTING_REQUEST_CODE = 10002;

    /** Per-session device sensor managers. */
    private static final ConcurrentHashMap<Integer, DeviceSensorManager> sSensorManagers =
            new ConcurrentHashMap<>();

    /** Per-session network monitors. */
    private static final ConcurrentHashMap<Integer, NetworkMonitor> sNetworkMonitors =
            new ConcurrentHashMap<>();

    /** Per-session error callbacks (registered by GameSession). */
    private static final ConcurrentHashMap<Integer, NativeErrorCallback> sErrorCallbacks =
            new ConcurrentHashMap<>();

    /** Handler for dispatching callbacks to the main thread. */
    private static final Handler sMainHandler = new Handler(Looper.getMainLooper());

    private NativeExports() {}

    // ==================== Error Notification (from native) ====================

    /**
     * Callback interface for native engine errors.
     * <p>
     * Implemented by the session owner (e.g. {@code GameSession}) to receive
     * fatal error notifications from the Rust engine.
     *
     * @hide
     */
    public interface NativeErrorCallback {
        /**
         * Called when a fatal native engine error occurs.
         * <p>
         * Always called on the <b>main thread</b>.
         *
         * @param errorCode Native error code (see {@code ErrorCode.NATIVE_*} constants)
         * @param message   Human-readable error message
         * @param detail    Detailed information (stack trace, etc.), may be empty
         */
        void onNativeError(int errorCode, String message, String detail);
    }

    /**
     * Register an error callback for a session.
     * <p>
     * Call during session creation. Only one callback per session is supported;
     * a subsequent registration replaces the previous one.
     *
     * @param sessionId The session ID
     * @param callback  The callback to receive error notifications
     * @hide
     */
    public static void registerErrorCallback(int sessionId, NativeErrorCallback callback) {
        if (callback != null) {
            sErrorCallbacks.put(sessionId, callback);
        }
    }

    /**
     * Unregister the error callback for a session.
     * <p>
     * Call during session shutdown.
     *
     * @param sessionId The session ID
     * @hide
     */
    public static void unregisterErrorCallback(int sessionId) {
        sErrorCallbacks.remove(sessionId);
    }

    /**
     * Called from native code (Rust) when a fatal engine error occurs.
     * <p>
     * This method is invoked from native threads (host thread, watchdog thread, etc.)
     * and dispatches the error to the registered callback on the <b>main thread</b>.
     * <p>
     * JNI signature: {@code (IILjava/lang/String;Ljava/lang/String;)V}
     *
     * @param hostId    Session/host ID
     * @param errorCode Native error code:
     *                  203 = OutOfMemory, 204 = JsExecutionTimeout,
     *                  205 = HostPanic, 206 = ANR,
     *                  207 = CodeSignatureInvalid, 208 = CodeIntegrityFailed
     * @param message   Human-readable error message
     * @param detail    Detailed error information (stack trace, context)
     */
    public static void onError(int hostId, int errorCode, String message, String detail) {
        NativeErrorCallback callback = sErrorCallbacks.get(hostId);
        if (callback != null) {
            // Dispatch to main thread — native calls may come from any thread
            sMainHandler.post(() -> {
                // Re-check: session may have been destroyed between post and dispatch
                NativeErrorCallback cb = sErrorCallbacks.get(hostId);
                if (cb != null) {
                    cb.onNativeError(errorCode,
                            message != null ? message : "Unknown native error",
                            detail != null ? detail : "");
                }
            });
        }
    }

    // ==================== Image Decoding ====================

    /**
     * Decode image bytes to RGBA using Android's BitmapFactory.
     * Returns [width_le32, height_le32, RGBA_bytes...] or null on failure.
     *
     * <p>ARGB_8888 internal pixel format is 0xAARRGGBB. On little-endian devices
     * (all Android), {@code copyPixelsToBuffer} writes bytes as B,G,R,A per pixel.
     * We perform an in-place BGRA→RGBA swizzle before returning so the caller
     * always receives true RGBA byte order.
     *
     * @param imageData Raw image file bytes (JPEG, PNG, BMP, etc.)
     * @return Packed byte array: 8-byte header (width + height as little-endian int32) + RGBA pixels, or null
     */
    public static byte[] decodeImageRgba(byte[] imageData) {
        if (imageData == null || imageData.length == 0) return null;

        BitmapFactory.Options opts = new BitmapFactory.Options();
        opts.inPreferredConfig = Bitmap.Config.ARGB_8888;
        // Decode as non-premultiplied so RGB channels are unmodified.
        // The GL pipeline handles alpha blending; premultiplied data
        // would corrupt colours when used with straight-alpha blending.
        opts.inPremultiplied = false;

        Bitmap bitmap = BitmapFactory.decodeByteArray(imageData, 0, imageData.length, opts);
        if (bitmap == null) return null;

        // Ensure ARGB_8888 config for consistent pixel format
        if (bitmap.getConfig() != Bitmap.Config.ARGB_8888) {
            Bitmap converted = bitmap.copy(Bitmap.Config.ARGB_8888, false);
            bitmap.recycle();
            if (converted == null) return null;
            bitmap = converted;
        }

        int w = bitmap.getWidth();
        int h = bitmap.getHeight();
        int pixelCount = w * h;

        // Single allocation: 8-byte header + pixel data (eliminates the
        // previous redundant pixelBuf allocation).
        ByteBuffer buf = ByteBuffer.allocate(8 + pixelCount * 4);
        buf.order(ByteOrder.LITTLE_ENDIAN);
        buf.putInt(w);
        buf.putInt(h);

        // copyPixelsToBuffer writes raw pixel bytes in native memory order.
        // Android's ARGB_8888 (backed by Skia kRGBA_8888) stores bytes as
        // R, G, B, A from low to high address — already RGBA byte order.
        // No swizzle needed.
        bitmap.copyPixelsToBuffer(buf);
        bitmap.recycle();

        return buf.array();
    }

    // ==================== File System ====================

    /**
     * Get the app's cache directory path.
     *
     * @return Cache directory path, or empty string if unavailable
     */
    public static String getCacheDirPath() {
        RuntimeContext context = RuntimeRegistry.getAny();
        if (context == null) {
            return "";
        }
        Activity activity = context.getActivity();
        if (activity == null) {
            Context appContext = AppContext.getOrNull();
            if (appContext != null) {
                File cacheDir = appContext.getCacheDir();
                return cacheDir != null ? cacheDir.getAbsolutePath() : "";
            }
            return "";
        }
        File cacheDir = activity.getCacheDir();
        return cacheDir != null ? cacheDir.getAbsolutePath() : "";
    }

    /**
     * Extract a zip file to target directory using Android's built-in java.util.zip.
     * <p>
     * Includes path traversal protection (zip slip prevention).
     *
     * @param zipFilePath Path to the zip file
     * @param targetPath  Destination directory
     * @return Number of files extracted on success, or error message prefixed with "ERR:"
     */
    public static String unzipFile(String zipFilePath, String targetPath) {
        if (zipFilePath == null || targetPath == null) {
            return "ERR:unzip:fail invalid arguments";
        }

        File zipFile = new File(zipFilePath);
        if (!zipFile.exists()) {
            return "ERR:unzip:fail file not found: " + zipFilePath;
        }

        File destDir = new File(targetPath);
        if (!destDir.exists() && !destDir.mkdirs()) {
            return "ERR:unzip:fail cannot create destination directory";
        }

        String canonicalDest;
        try {
            canonicalDest = destDir.getCanonicalPath();
        } catch (java.io.IOException e) {
            return "ERR:unzip:fail cannot resolve destination path";
        }

        int fileCount = 0;
        try (java.util.zip.ZipInputStream zis = new java.util.zip.ZipInputStream(
                new java.io.BufferedInputStream(new java.io.FileInputStream(zipFile), 65536))) {

            java.util.zip.ZipEntry entry;
            byte[] buffer = new byte[8192];

            while ((entry = zis.getNextEntry()) != null) {
                File outFile = new File(destDir, entry.getName());

                // Security: path traversal protection (zip slip)
                String canonicalPath = outFile.getCanonicalPath();
                if (!canonicalPath.startsWith(canonicalDest + File.separator)
                        && !canonicalPath.equals(canonicalDest)) {
                    zis.closeEntry();
                    return "ERR:unzip:fail path traversal detected: " + entry.getName();
                }

                if (entry.isDirectory()) {
                    if (!outFile.exists()) {
                        outFile.mkdirs();
                    }
                } else {
                    // Ensure parent directories exist
                    File parent = outFile.getParentFile();
                    if (parent != null && !parent.exists()) {
                        parent.mkdirs();
                    }

                    try (java.io.FileOutputStream fos = new java.io.FileOutputStream(outFile)) {
                        int len;
                        while ((len = zis.read(buffer)) > 0) {
                            fos.write(buffer, 0, len);
                        }
                    }
                    fileCount++;
                }
                zis.closeEntry();
            }
        } catch (java.io.IOException e) {
            return "ERR:unzip:fail " + (e.getMessage() != null ? e.getMessage() : "IO error");
        }

        return String.valueOf(fileCount);
    }

    // ==================== Charset Encoding (GBK) ====================

    /**
     * Encode a string to GBK bytes using Android's built-in java.nio.charset.Charset.
     * Available on all Android API levels (API 1+).
     *
     * @param data The string to encode
     * @return GBK-encoded bytes, or null on error
     */
    public static byte[] encodeGbk(String data) {
        if (data == null) return null;
        try {
            return data.getBytes("GBK");
        } catch (java.io.UnsupportedEncodingException e) {
            return null;
        }
    }

    /**
     * Decode GBK bytes to a string using Android's built-in java.nio.charset.Charset.
     * Available on all Android API levels (API 1+).
     *
     * @param data The GBK-encoded bytes
     * @return Decoded string, or null on error
     */
    public static String decodeGbk(byte[] data) {
        if (data == null) return null;
        try {
            return new String(data, "GBK");
        } catch (java.io.UnsupportedEncodingException e) {
            return null;
        }
    }

    // ==================== System Settings ====================

    /**
     * Open system Bluetooth settings.
     *
     * @param sessionId The session ID to receive the callback
     */
    public static void openSystemBluetoothSetting(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) {
            NativeMethods.onBluetoothSettingResult(sessionId, false);
            return;
        }

        Activity activity = context.getActivity();
        if (activity == null) {
            NativeMethods.onBluetoothSettingResult(sessionId, false);
            return;
        }

        try {
            Intent intent = new Intent(Settings.ACTION_BLUETOOTH_SETTINGS);
            activity.startActivityForResult(intent, BLUETOOTH_SETTING_REQUEST_CODE);
        } catch (Exception e) {
            NativeMethods.onBluetoothSettingResult(sessionId, false);
        }
    }

    /**
     * Open app authorization (permission) settings page.
     *
     * @param sessionId The session ID to receive the callback
     */
    public static void openAppAuthorizeSetting(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) {
            NativeMethods.onAppAuthorizeSettingResult(sessionId, -1);
            return;
        }

        Activity activity = context.getActivity();
        if (activity == null) {
            NativeMethods.onAppAuthorizeSettingResult(sessionId, -1);
            return;
        }

        try {
            Intent intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
            Uri uri = Uri.fromParts("package", activity.getPackageName(), null);
            intent.setData(uri);
            activity.startActivityForResult(intent, APP_AUTHORIZE_SETTING_REQUEST_CODE);
        } catch (Exception e) {
            NativeMethods.onAppAuthorizeSettingResult(sessionId, -1);
        }
    }

    // ==================== Window Information ====================

    /**
     * Get window information as a packed byte array.
     * <p>
     * Layout (52 bytes total):
     * - [0-3]:   windowWidth (int)
     * - [4-7]:   windowHeight (int)
     * - [8-11]:  screenWidth (int)
     * - [12-15]: screenHeight (int)
     * - [16-19]: statusBarHeight (int)
     * - [20-23]: pixelRatio * 1000 (int)
     * - [24-27]: screenTop (int)
     * - [28-31]: reserved
     * - [32-35]: reserved
     * - [36-39]: safeAreaLeft (int)
     * - [40-43]: safeAreaTop (int)
     * - [44-47]: safeAreaRight (int)
     * - [48-51]: safeAreaBottom (int)
     *
     * @param sessionId The session ID
     * @return Packed byte array, or null if unavailable
     */
    public static byte[] getWindowInfoBytes(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) {
            return null;
        }

        Activity activity = context.getActivity();
        if (activity == null) {
            return null;
        }

        try {
            // Use DisplayCompat for API 21+ compatibility
            int screenWidth = DisplayCompat.getScreenWidth(activity);
            int screenHeight = DisplayCompat.getScreenHeight(activity);
            float density = DisplayCompat.getDensity(activity);
            int statusBarHeight = DisplayCompat.getStatusBarHeight(activity);

            // Window dimensions (use decor view if available)
            int windowWidth = screenWidth;
            int windowHeight = screenHeight;
            int screenTop = 0;

            try {
                android.view.View decorView = activity.getWindow().getDecorView();
                if (decorView.getWidth() > 0) {
                    windowWidth = decorView.getWidth();
                    windowHeight = decorView.getHeight();
                }
                int[] location = new int[2];
                decorView.getLocationOnScreen(location);
                screenTop = location[1];
            } catch (Exception ignored) {
            }

            // Safe area insets using compat layer
            DisplayCompat.SafeAreaInsets safeArea = DisplayCompat.getSafeAreaInsets(activity);

            ByteBuffer buffer = ByteBuffer.allocate(52);
            buffer.order(ByteOrder.nativeOrder());
            buffer.putInt(windowWidth);     // 0-3
            buffer.putInt(windowHeight);    // 4-7
            buffer.putInt(screenWidth);     // 8-11
            buffer.putInt(screenHeight);    // 12-15
            buffer.putInt(statusBarHeight); // 16-19
            buffer.putInt((int) (density * 1000)); // 20-23
            buffer.putInt(screenTop);       // 24-27
            buffer.putInt(0);               // 28-31: reserved
            buffer.putInt(0);               // 32-35: reserved
            buffer.putInt(safeArea.left);   // 36-39
            buffer.putInt(safeArea.top);    // 40-43
            buffer.putInt(safeArea.right);  // 44-47
            buffer.putInt(safeArea.bottom); // 48-51

            return buffer.array();
        } catch (Exception e) {
            return null;
        }
    }

    // ==================== System Settings Info ====================

    /**
     * Get system settings as a packed byte array.
     * <p>
     * Layout (4 bytes):
     * - [0]: bluetoothEnabled (0/1)
     * - [1]: locationEnabled (0/1)
     * - [2]: wifiEnabled (0/1)
     * - [3]: orientation (0=unknown, 1=portrait, 2=landscape)
     *
     * @return Packed byte array
     */
    public static byte[] getSystemSettingInfoBytes() {
        RuntimeContext context = RuntimeRegistry.getAny();
        Activity activity = context != null ? context.getActivity() : null;
        Context appContext = activity != null ? activity : AppContext.getOrNull();

        boolean isLandscape = activity != null && DisplayCompat.isLandscape(activity);
        return SystemSettings.toBytes(appContext, isLandscape);
    }

    // ==================== Device Information ====================

    /**
     * Get device information as JSON string.
     *
     * @return JSON string with device info
     */
    public static String getDeviceInfoJson() {
        Context appContext = AppContext.getOrNull();
        return DeviceInfo.toJson(appContext);
    }

    // ==================== Battery ====================

    /**
     * Get battery info as JSON string.
     *
     * @return JSON string with battery level, charging status, and low power mode
     */
    public static String getBatteryInfoJson() {
        Context appContext = AppContext.getOrNull();
        return BatteryInfo.toJson(appContext);
    }

    // ==================== Vibration ====================

    /**
     * Trigger a short vibration (15ms).
     *
     * @param type Vibration type: "heavy", "medium", or "light"
     * @return 0 on success, -1 if unavailable, -2 if type not supported
     */
    public static int vibrateShort(String type) {
        Context appContext = AppContext.getOrNull();
        return Vibrator.vibrateShort(appContext, type);
    }

    /**
     * Trigger a long vibration (400ms).
     *
     * @return 0 on success, -1 if unavailable
     */
    public static int vibrateLong() {
        Context appContext = AppContext.getOrNull();
        return Vibrator.vibrateLong(appContext);
    }

    // ==================== Screen ====================

    /**
     * Get current screen brightness.
     *
     * @param sessionId The session ID
     * @return Brightness value 0.0-1.0, or -1 if following system
     */
    public static float getScreenBrightness(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return -1f;
        Activity activity = context.getActivity();
        return ScreenBrightness.getBrightness(activity);
    }

    /**
     * Set screen brightness.
     *
     * @param sessionId The session ID
     * @param value     Brightness value (0.0-1.0) or -1 for system default
     * @return 0 on success, -1 on failure
     */
    public static int setScreenBrightness(int sessionId, float value) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return -1;
        Activity activity = context.getActivity();
        return ScreenBrightness.setBrightness(activity, value);
    }

    /**
     * Set whether to keep screen on.
     *
     * @param sessionId    The session ID
     * @param keepScreenOn true to keep screen on
     * @return 0 on success, -1 on failure
     */
    public static int setKeepScreenOn(int sessionId, boolean keepScreenOn) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return -1;
        Activity activity = context.getActivity();
        return ScreenBrightness.setKeepScreenOn(activity, keepScreenOn);
    }

    /**
     * Set device orientation (landscape or portrait).
     *
     * @param sessionId The session ID
     * @param value     "landscape" or "portrait"
     * @return 0 on success, -1 on failure, -2 if value is invalid
     */
    public static int setDeviceOrientation(int sessionId, String value) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return -1;
        Activity activity = context.getActivity();
        return ScreenBrightness.setDeviceOrientation(activity, value);
    }

    // ==================== UI Interaction ====================

    /**
     * Show a toast overlay.
     *
     * @param sessionId The session ID
     * @param json      JSON params: {title, icon, duration, mask}
     */
    public static void showToast(int sessionId, String json) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return;
        Activity activity = context.getActivity();
        if (activity == null) return;
        InteractionUI.showToast(activity, json);
    }

    /**
     * Hide the current toast.
     *
     * @param sessionId The session ID
     */
    public static void hideToast(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return;
        Activity activity = context.getActivity();
        if (activity == null) return;
        InteractionUI.hideToast(activity);
    }

    /**
     * Show a modal dialog.
     *
     * @param sessionId The session ID
     * @param json      JSON params: {title, content, showCancel, cancelText, confirmText, cancelColor, confirmColor}
     */
    public static void showModal(int sessionId, String json) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) {
            NativeMethods.onModalResult(sessionId, 0, 1);
            return;
        }
        Activity activity = context.getActivity();
        if (activity == null) {
            NativeMethods.onModalResult(sessionId, 0, 1);
            return;
        }
        InteractionUI.showModal(activity, sessionId, json);
    }

    /**
     * Show a loading overlay.
     *
     * @param sessionId The session ID
     * @param json      JSON params: {title, mask}
     */
    public static void showLoading(int sessionId, String json) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return;
        Activity activity = context.getActivity();
        if (activity == null) return;
        InteractionUI.showLoading(activity, json);
    }

    /**
     * Hide the current loading overlay.
     *
     * @param sessionId The session ID
     */
    public static void hideLoading(int sessionId) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return;
        Activity activity = context.getActivity();
        if (activity == null) return;
        InteractionUI.hideLoading(activity);
    }

    /**
     * Show an action sheet.
     *
     * @param sessionId The session ID
     * @param json      JSON params: {alertText, itemList, itemColor}
     */
    public static void showActionSheet(int sessionId, String json) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) {
            NativeMethods.onActionSheetResult(sessionId, -1);
            return;
        }
        Activity activity = context.getActivity();
        if (activity == null) {
            NativeMethods.onActionSheetResult(sessionId, -1);
            return;
        }
        InteractionUI.showActionSheet(activity, sessionId, json);
    }

    // ==================== Permissions ====================

    /**
     * Get app authorization settings as JSON string.
     *
     * @return JSON string with permission states
     */
    public static String getAppAuthorizationSettingJson() {
        RuntimeContext runtimeContext = RuntimeRegistry.getAny();
        Context context = runtimeContext != null ? runtimeContext.getActivity() : null;
        if (context == null) {
            context = AppContext.getOrNull();
        }
        return Permissions.toJson(context);
    }

    // ==================== Device Sensor ====================

    /**
     * Get or create a DeviceSensorManager for the given session.
     *
     * @param sessionId The session ID
     * @return DeviceSensorManager, or null if context is unavailable
     */
    private static DeviceSensorManager getOrCreateSensorManager(int sessionId) {
        DeviceSensorManager existing = sSensorManagers.get(sessionId);
        if (existing != null) return existing;

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) return null;
        Activity activity = ctx.getActivity();
        if (activity == null) return null;

        DeviceSensorManager mgr = new DeviceSensorManager(sessionId, activity);
        sSensorManagers.put(sessionId, mgr);
        return mgr;
    }

    /**
     * Start listening for device motion (rotation vector) events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     * @param interval  "game", "ui", or "normal"
     */
    public static void startDeviceMotionListening(int sessionId, String interval) {
        DeviceSensorManager mgr = getOrCreateSensorManager(sessionId);
        if (mgr != null) {
            mgr.startDeviceMotionListening(interval);
        }
    }

    /**
     * Stop listening for device motion events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void stopDeviceMotionListening(int sessionId) {
        DeviceSensorManager mgr = sSensorManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopDeviceMotionListening();
        }
    }

    /**
     * Start listening for gyroscope events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     * @param interval  "game", "ui", or "normal"
     */
    public static void startGyroscope(int sessionId, String interval) {
        DeviceSensorManager mgr = getOrCreateSensorManager(sessionId);
        if (mgr != null) {
            mgr.startGyroscope(interval);
        }
    }

    /**
     * Stop listening for gyroscope events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void stopGyroscope(int sessionId) {
        DeviceSensorManager mgr = sSensorManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopGyroscope();
        }
    }

    /**
     * Start listening for compass events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void startCompass(int sessionId) {
        DeviceSensorManager mgr = getOrCreateSensorManager(sessionId);
        if (mgr != null) {
            mgr.startCompass();
        }
    }

    /**
     * Stop listening for compass events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void stopCompass(int sessionId) {
        DeviceSensorManager mgr = sSensorManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopCompass();
        }
    }

    /**
     * Start listening for accelerometer events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     * @param interval  "game", "ui", or "normal"
     */
    public static void startAccelerometer(int sessionId, String interval) {
        DeviceSensorManager mgr = getOrCreateSensorManager(sessionId);
        if (mgr != null) {
            mgr.startAccelerometer(interval);
        }
    }

    /**
     * Stop listening for accelerometer events.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void stopAccelerometer(int sessionId) {
        DeviceSensorManager mgr = sSensorManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopAccelerometer();
        }
    }

    /**
     * Clean up sensor resources for a session. Call on session shutdown.
     *
     * @param sessionId The session ID
     */
    public static void destroySensorManager(int sessionId) {
        DeviceSensorManager mgr = sSensorManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Network ====================

    /**
     * Get or create a NetworkMonitor for the given session.
     *
     * @param sessionId The session ID
     * @return NetworkMonitor, or null if context is unavailable
     */
    private static NetworkMonitor getOrCreateNetworkMonitor(int sessionId) {
        NetworkMonitor existing = sNetworkMonitors.get(sessionId);
        if (existing != null) return existing;

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) return null;
        Activity activity = ctx.getActivity();
        if (activity == null) return null;

        NetworkMonitor mgr = new NetworkMonitor(sessionId, activity);
        sNetworkMonitors.put(sessionId, mgr);
        return mgr;
    }

    /**
     * Start monitoring network status changes.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void startNetworkMonitoring(int sessionId) {
        NetworkMonitor mgr = getOrCreateNetworkMonitor(sessionId);
        if (mgr != null) {
            mgr.startMonitoring();
        }
    }

    /**
     * Stop monitoring network status changes.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     */
    public static void stopNetworkMonitoring(int sessionId) {
        NetworkMonitor mgr = sNetworkMonitors.get(sessionId);
        if (mgr != null) {
            mgr.stopMonitoring();
        }
    }

    /**
     * Get current network type as JSON string.
     *
     * @param sessionId The session ID
     * @return JSON string with networkType, isConnected, signalStrength, hasSystemProxy, weakNet
     */
    public static String getNetworkTypeJson(int sessionId) {
        NetworkMonitor mgr = getOrCreateNetworkMonitor(sessionId);
        if (mgr == null) {
            return "{\"networkType\":\"none\",\"isConnected\":false,\"signalStrength\":0,\"hasSystemProxy\":false,\"weakNet\":false}";
        }

        NetworkMonitor.NetworkStatus status = mgr.getNetworkStatus();
        if (status.error != null) {
            return String.format(
                "{\"_error\":{\"errMsg\":\"%s\"}}",
                status.error
            );
        }

        return String.format(
                "{\"networkType\":\"%s\",\"isConnected\":%s,\"signalStrength\":%d,\"hasSystemProxy\":%s,\"weakNet\":%s}",
                status.networkType,
                status.isConnected,
                status.signalStrength,
                status.hasSystemProxy,
                status.weakNet
        );
    }

    /**
     * Get local IP address as JSON string.
     *
     * @return JSON string with localip and netmask
     */
    public static String getLocalIPAddressJson() {
        NetworkMonitor.LocalIPInfo info = NetworkMonitor.getLocalIPAddress();
        if (info.error != null) {
            return String.format(
                "{\"_error\":{\"errMsg\":\"%s\"}}",
                info.error
            );
        }

        return String.format(
                "{\"localip\":\"%s\",\"netmask\":\"%s\"}",
                info.localip,
                info.netmask
        );
    }

    /**
     * Clean up network monitor resources for a session. Call on session shutdown.
     *
     * @param sessionId The session ID
     */
    public static void destroyNetworkMonitor(int sessionId) {
        NetworkMonitor mgr = sNetworkMonitors.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Audio Platform ====================

    /**
     * Set inner audio options for audio focus and routing.
     *
     * @param sessionId      The session ID
     * @param mixWithOther   If true, duck other audio instead of taking exclusive focus
     * @param obeyMuteSwitch If true, respect the device ringer/mute mode
     * @param speakerOn      If true, route audio output to speaker
     */
    public static void setInnerAudioOption(int sessionId, boolean mixWithOther,
                                           boolean obeyMuteSwitch, boolean speakerOn) {
        RuntimeContext context = RuntimeRegistry.get(sessionId);
        if (context == null) return;
        Activity activity = context.getActivity();
        if (activity == null) return;

        try {
            android.media.AudioManager audioManager =
                    (android.media.AudioManager) activity.getSystemService(Context.AUDIO_SERVICE);
            if (audioManager == null) return;

            // Audio focus: GAIN_TRANSIENT_MAY_DUCK allows mixing, GAIN takes exclusive focus
            int focusGain = mixWithOther
                    ? android.media.AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK
                    : android.media.AudioManager.AUDIOFOCUS_GAIN;
            audioManager.requestAudioFocus(null,
                    android.media.AudioManager.STREAM_MUSIC, focusGain);

            // Speaker routing
            audioManager.setSpeakerphoneOn(speakerOn);

            // obeyMuteSwitch: adjust stream type behavior
            // When obeyMuteSwitch is false, use STREAM_MUSIC which ignores ringer mode
            // When true, the app should check ringer mode before playing
            // This is stored and checked at playback time by the audio engine
        } catch (Exception e) {
            // Silently fail - audio options are best-effort
        }
    }

    /**
     * Get available audio input sources.
     * Returns a comma-separated string of supported audio source identifiers
     * matching RecorderManager.start() audioSource param values.
     *
     * @param sessionId The session ID
     * @return Comma-separated audio source identifiers (e.g., "auto,buildInMic,mic,camcorder,voice_recognition,voice_communication")
     */
    public static String getAvailableAudioSources(int sessionId) {
        StringBuilder sb = new StringBuilder();
        // "auto" is always available
        sb.append("auto");

        // Check each MediaRecorder.AudioSource constant
        // DEFAULT (0) maps to "buildInMic"
        sb.append(",buildInMic");

        // MIC (1) - standard microphone
        sb.append(",mic");

        // CAMCORDER (5) - microphone tuned for video recording
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.HONEYCOMB) {
            sb.append(",camcorder");
        }

        // VOICE_RECOGNITION (6) - tuned for voice recognition
        sb.append(",voice_recognition");

        // VOICE_COMMUNICATION (7) - tuned for VoIP, includes echo cancellation
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.HONEYCOMB) {
            sb.append(",voice_communication");
        }

        return sb.toString();
    }

    // ==================== Clipboard ====================

    /**
     * Set clipboard content.
     * Shows a toast "内容已复制" for ~1.5 seconds.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     * @param data      The text data to copy
     * @return 0 on success, -1 on failure
     */
    public static int setClipboardData(int sessionId, String data) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) return -1;
        Activity activity = ctx.getActivity();
        if (activity == null) return -1;
        return Clipboard.setClipboardData(activity, data);
    }

    /**
     * Get clipboard content.
     * Called from native code via JNI.
     *
     * @param sessionId The session ID
     * @return The clipboard text content, or empty string if unavailable
     */
    public static String getClipboardData(int sessionId) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) return "";
        Activity activity = ctx.getActivity();
        if (activity == null) return "";
        return Clipboard.getClipboardData(activity);
    }

    // ==================== Recorder ====================

    /** Per-session recorder managers. */
    private static final ConcurrentHashMap<Integer, AudioRecorderManager> sRecorderManagers =
            new ConcurrentHashMap<>();

    /**
     * Start recording with the given options.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON string with recording options:
     *                    duration, sampleRate, numberOfChannels, encodeBitRate,
     *                    format, frameSize, audioSource
     */
    public static void recorderStart(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            NativeMethods.onRecorderEvent(sessionId, "error",
                    "{\"errMsg\":\"recorderManager.start:fail no context\"}");
            return;
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            NativeMethods.onRecorderEvent(sessionId, "error",
                    "{\"errMsg\":\"recorderManager.start:fail no activity\"}");
            return;
        }

        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr == null) {
            mgr = new AudioRecorderManager(sessionId, activity);
            sRecorderManagers.put(sessionId, mgr);
        }
        mgr.start(optionsJson);
    }

    /**
     * Pause recording.
     *
     * @param sessionId The session ID
     */
    public static void recorderPause(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.pause();
        }
    }

    /**
     * Resume recording after pause.
     *
     * @param sessionId The session ID
     */
    public static void recorderResume(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.resume();
        }
    }

    /**
     * Stop recording.
     *
     * @param sessionId The session ID
     */
    public static void recorderStop(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.stop();
        }
    }

    /**
     * Clean up recorder resources for a session. Call on session shutdown.
     *
     * @param sessionId The session ID
     */
    public static void destroyRecorderManager(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Camera ====================

    /** Per-session camera managers, keyed by "sessionId:cameraId". */
    private static final ConcurrentHashMap<String, CameraManager> sCameraManagers =
            new ConcurrentHashMap<>();

    private static String cameraKey(int sessionId, int cameraId) {
        return sessionId + ":" + cameraId;
    }

    /**
     * Create a camera instance.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with keys: cameraId, pos, flash, size
     * @return JSON result: {"cameraId": <id>} or error JSON
     */
    public static String cameraCreate(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            return "{\"_error\":{\"errMsg\":\"createCamera:fail no context\"}}";
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            return "{\"_error\":{\"errMsg\":\"createCamera:fail no activity\"}}";
        }

        // Extract cameraId from options
        int cameraId = 0;
        try {
            org.json.JSONObject opts = new org.json.JSONObject(optionsJson);
            cameraId = opts.optInt("cameraId", 0);
        } catch (Exception ignored) {}

        String key = cameraKey(sessionId, cameraId);
        CameraManager existing = sCameraManagers.get(key);
        if (existing != null) {
            existing.destroy();
            sCameraManagers.remove(key);
        }

        CameraManager mgr = new CameraManager(sessionId, cameraId, activity);
        String result = mgr.create(optionsJson);
        sCameraManagers.put(key, mgr);
        return result;
    }

    /**
     * Destroy a camera instance.
     *
     * @param sessionId The session ID
     * @param cameraId  The camera instance ID
     */
    public static void cameraDestroy(int sessionId, int cameraId) {
        String key = cameraKey(sessionId, cameraId);
        CameraManager mgr = sCameraManagers.remove(key);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    /**
     * Take a photo with the camera.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with keys: cameraId, quality
     * @return JSON result or error JSON
     */
    public static String cameraTakePhoto(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.takePhoto:fail camera not found\"}}";
        }
        return mgr.takePhoto(optionsJson);
    }

    /**
     * Start video recording.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with keys: cameraId, timeout
     * @return JSON result or error JSON
     */
    public static String cameraStartRecord(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.startRecord:fail camera not found\"}}";
        }
        return mgr.startRecord(optionsJson);
    }

    /**
     * Stop video recording.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with keys: cameraId, compressed
     * @return JSON result or error JSON
     */
    public static String cameraStopRecord(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.stopRecord:fail camera not found\"}}";
        }
        return mgr.stopRecord(optionsJson);
    }

    /**
     * Set camera zoom level.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with keys: cameraId, zoom
     * @return JSON result or error JSON
     */
    public static String cameraSetZoom(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.setZoom:fail camera not found\"}}";
        }
        return mgr.setZoom(optionsJson);
    }

    /**
     * Start listening for camera frame changes.
     *
     * @param sessionId The session ID
     * @param cameraId  The camera instance ID
     */
    public static void cameraListenFrameChange(int sessionId, int cameraId) {
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr != null) {
            mgr.listenFrameChange();
        }
    }

    /**
     * Stop listening for camera frame changes.
     *
     * @param sessionId The session ID
     * @param cameraId  The camera instance ID
     */
    public static void cameraCloseFrameChange(int sessionId, int cameraId) {
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr != null) {
            mgr.closeFrameChange();
        }
    }

    /**
     * Clean up all camera resources for a session. Call on session shutdown.
     *
     * @param sessionId The session ID
     */
    public static void destroyCameraManagers(int sessionId) {
        String prefix = sessionId + ":";
        for (String key : sCameraManagers.keySet()) {
            if (key.startsWith(prefix)) {
                CameraManager mgr = sCameraManagers.remove(key);
                if (mgr != null) {
                    mgr.destroy();
                }
            }
        }
    }

    private static int extractCameraId(String optionsJson) {
        if (optionsJson == null) return 0;
        try {
            org.json.JSONObject opts = new org.json.JSONObject(optionsJson);
            return opts.optInt("cameraId", 0);
        } catch (Exception e) {
            return 0;
        }
    }
}
