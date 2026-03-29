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
import com.migo.runtime.internal.platform.BluetoothManager;
import com.migo.runtime.internal.platform.CameraManager;
import com.migo.runtime.internal.platform.KeyboardManager;
import com.migo.runtime.internal.platform.ImageApiManager;
import com.migo.runtime.internal.platform.LocationProvider;
import com.migo.runtime.internal.platform.ScanCodeManager;
import com.migo.runtime.internal.platform.ScreenCaptureObserver;
import com.migo.runtime.internal.platform.VideoManager;
import com.migo.runtime.callback.AuthHandler;
import com.migo.runtime.callback.GameLogHandler;
import com.migo.runtime.callback.SubpackageHandler;

import com.migo.runtime.GameSession;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.os.Handler;
import android.os.Looper;

import org.json.JSONException;
import org.json.JSONObject;

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

    // Locks for double-checked locking in getOrCreate methods
    private static final Object sSensorLock = new Object();
    private static final Object sNetworkLock = new Object();
    private static final Object sCaptureLock = new Object();
    private static final Object sRecorderLock = new Object();
    private static final Object sKeyboardLock = new Object();
    private static final Object sBluetoothLock = new Object();
    private static final Object sImageApiLock = new Object();
    private static final Object sScanCodeLock = new Object();
    private static final Object sVideoLock = new Object();

    /** Per-session device sensor managers. */
    private static final ConcurrentHashMap<Integer, DeviceSensorManager> sSensorManagers =
            new ConcurrentHashMap<>();

    /** Per-session network monitors. */
    private static final ConcurrentHashMap<Integer, NetworkMonitor> sNetworkMonitors =
            new ConcurrentHashMap<>();

    /** Per-session screen capture observers. */
    private static final ConcurrentHashMap<Integer, ScreenCaptureObserver> sCaptureObservers =
            new ConcurrentHashMap<>();

    /** Per-session error callbacks (registered by GameSession). */
    private static final ConcurrentHashMap<Integer, NativeErrorCallback> sErrorCallbacks =
            new ConcurrentHashMap<>();

    /** Per-session GameSession references for lifecycle callbacks. */
    private static final ConcurrentHashMap<Integer, GameSession> sSessions =
            new ConcurrentHashMap<>();

    /** Per-session auth handlers set via GameSession API. */
    private static final ConcurrentHashMap<Integer, AuthHandler> sAuthHandlers =
            new ConcurrentHashMap<>();

    /** Per-session game log handlers set via GameSession API. */
    private static final ConcurrentHashMap<Integer, GameLogHandler> sGameLogHandlers =
            new ConcurrentHashMap<>();

    /** Per-session subpackage handlers set via GameSession API. */
    private static final ConcurrentHashMap<Integer, SubpackageHandler> sSubpackageHandlers =
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

        /**
         * Called when the mini program exits.
         * <p>
         * Always called on the <b>main thread</b>.
         */
        void onExit();
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
     * Register a GameSession for lifecycle callbacks from native.
     * @hide
     */
    public static void registerSession(int sessionId, GameSession session) {
        if (session != null) {
            sSessions.put(sessionId, session);
        }
    }

    /**
     * Unregister a GameSession.
     * @hide
     */
    public static void unregisterSession(int sessionId) {
        sSessions.remove(sessionId);
    }

    /**
     * Called from native code (Rust) when the game module has been loaded.
     * <p>
     * JNI signature: {@code (I)V}
     *
     * @param hostId Session/host ID
     */
    public static void onGameReady(int hostId) {
        sMainHandler.post(() -> {
            GameSession session = sSessions.get(hostId);
            if (session != null) {
                session.notifyGameReady();
            }
        });
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

    /**
     * Called from native code (Rust) when the mini program exits.
     * <p>
     * JNI signature: {@code (I)V}
     *
     * @param hostId    Session/host ID
     */
    public static void onExit(int hostId) {
        NativeErrorCallback callback = sErrorCallbacks.get(hostId);
        if (callback != null) {
            // Dispatch to main thread
            sMainHandler.post(() -> {
                NativeErrorCallback cb = sErrorCallbacks.get(hostId);
                if (cb != null) {
                    cb.onExit();
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
        long pixelCountLong = (long) w * (long) h;
        if (pixelCountLong > Integer.MAX_VALUE / 4) {
            bitmap.recycle();
            return null;  // Image too large
        }
        int pixelCount = (int) pixelCountLong;

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
            ResultProxyActivity.launch(activity, intent, BLUETOOTH_SETTING_REQUEST_CODE,
                    (requestCode, resultCode, data) -> {
                        android.bluetooth.BluetoothAdapter adapter =
                                android.bluetooth.BluetoothAdapter.getDefaultAdapter();
                        boolean enabled = adapter != null && adapter.isEnabled();
                        NativeMethods.onBluetoothSettingResult(sessionId, enabled);
                    });
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
            ResultProxyActivity.launch(activity, intent, APP_AUTHORIZE_SETTING_REQUEST_CODE,
                    (requestCode, resultCode, data) ->
                            NativeMethods.onAppAuthorizeSettingResult(sessionId, 0));
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
        return DisplayCompat.setDeviceOrientation(activity, value);
    }

    // ==================== Debug ====================

    /**
     * Set whether debug mode is enabled at runtime.
     *
     * @param sessionId   The session ID
     * @param enableDebug true to enable debug, false to disable
     * @return 0 on success, -1 on failure
     */
    public static int setEnableDebug(int sessionId, boolean enableDebug) {
        GameSession session = sSessions.get(sessionId);
        if (session == null) return -1;
        sMainHandler.post(() -> session.setDebugEnabled(enableDebug));
        return 0;
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
        synchronized (sSensorLock) {
            existing = sSensorManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            DeviceSensorManager mgr = new DeviceSensorManager(sessionId, activity);
            sSensorManagers.put(sessionId, mgr);
            return mgr;
        }
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
        synchronized (sNetworkLock) {
            existing = sNetworkMonitors.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            NetworkMonitor mgr = new NetworkMonitor(sessionId, activity);
            sNetworkMonitors.put(sessionId, mgr);
            return mgr;
        }
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

    // ==================== Screen Capture ====================

    /**
     * Start observing user screenshot events (lazy, called from JS onUserCaptureScreen).
     *
     * @param sessionId The session ID
     */
    public static void startCaptureScreen(int sessionId) {
        ScreenCaptureObserver observer = sCaptureObservers.get(sessionId);
        if (observer == null) {
            synchronized (sCaptureLock) {
                observer = sCaptureObservers.get(sessionId);
                if (observer == null) {
                    RuntimeContext ctx = RuntimeRegistry.get(sessionId);
                    if (ctx == null) return;
                    Activity activity = ctx.getActivity();
                    Context appCtx = activity != null ? activity : AppContext.getOrNull();
                    if (appCtx == null) return;
                    observer = new ScreenCaptureObserver(sessionId, appCtx);
                    sCaptureObservers.put(sessionId, observer);
                }
            }
        }
        observer.start();
    }

    /**
     * Stop observing user screenshot events (called from JS offUserCaptureScreen).
     *
     * @param sessionId The session ID
     */
    public static void stopCaptureScreen(int sessionId) {
        ScreenCaptureObserver observer = sCaptureObservers.get(sessionId);
        if (observer != null) {
            observer.stop();
        }
    }

    /**
     * Destroy the screen capture observer for a session.
     *
     * @param sessionId The session ID
     * @hide
     */
    public static void destroyCaptureObserver(int sessionId) {
        ScreenCaptureObserver observer = sCaptureObservers.remove(sessionId);
        if (observer != null) {
            observer.destroy();
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
            try {
                JSONObject err = new JSONObject();
                JSONObject inner = new JSONObject();
                inner.put("errMsg", status.error);
                err.put("_error", inner);
                return err.toString();
            } catch (JSONException e) {
                return "{}";
            }
        }

        try {
            JSONObject json = new JSONObject();
            json.put("networkType", status.networkType);
            json.put("isConnected", status.isConnected);
            json.put("signalStrength", status.signalStrength);
            json.put("hasSystemProxy", status.hasSystemProxy);
            json.put("weakNet", status.weakNet);
            return json.toString();
        } catch (JSONException e) {
            return "{}";
        }
    }

    /**
     * Get local IP address as JSON string.
     *
     * @return JSON string with localip and netmask
     */
    public static String getLocalIPAddressJson() {
        NetworkMonitor.LocalIPInfo info = NetworkMonitor.getLocalIPAddress();
        if (info.error != null) {
            try {
                JSONObject err = new JSONObject();
                JSONObject inner = new JSONObject();
                inner.put("errMsg", info.error);
                err.put("_error", inner);
                return err.toString();
            } catch (JSONException e) {
                return "{}";
            }
        }

        try {
            JSONObject json = new JSONObject();
            json.put("localip", info.localip);
            json.put("netmask", info.netmask);
            return json.toString();
        } catch (JSONException e) {
            return "{}";
        }
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
            synchronized (sRecorderLock) {
                mgr = sRecorderManagers.get(sessionId);
                if (mgr == null) {
                    mgr = new AudioRecorderManager(sessionId, activity);
                    sRecorderManagers.put(sessionId, mgr);
                }
            }
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
     * @param optionsJson JSON with keys: cameraId, x, y, width, height, devicePosition, flash, size
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
     * @param optionsJson JSON with keys: cameraId
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

    /**
     * Extract the "requestId" field from a JSON string as a raw number string.
     * Returns null if not present or not parseable.
     */
    private static String extractRequestId(String optionsJson) {
        if (optionsJson == null) return null;
        try {
            org.json.JSONObject opts = new org.json.JSONObject(optionsJson);
            if (opts.has("requestId")) {
                return String.valueOf(opts.getInt("requestId"));
            }
        } catch (Exception e) {
            // ignore
        }
        return null;
    }

    private static String buildDeferredErrorResult(String requestId, String errMsg, int errCode) {
        String ridField = requestId != null ? "\"requestId\":" + requestId + "," : "";
        return "{" + ridField + "\"error\":\"" + errMsg + "\",\"errCode\":" + errCode + "}";
    }

    // ==================== Keyboard ====================

    /** Per-session Keyboard managers. */
    private static final ConcurrentHashMap<Integer, KeyboardManager> sKeyboardManagers =
            new ConcurrentHashMap<>();

    private static KeyboardManager getOrCreateKeyboardManager(int sessionId) {
        KeyboardManager existing = sKeyboardManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sKeyboardLock) {
            existing = sKeyboardManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            KeyboardManager mgr = new KeyboardManager(sessionId, activity);
            sKeyboardManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void keyboardShow(int sessionId, String optionsJson) {
        KeyboardManager mgr = getOrCreateKeyboardManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.show(optionsJson);
    }

    public static void keyboardHide(int sessionId) {
        KeyboardManager mgr = sKeyboardManagers.get(sessionId);
        if (mgr != null) {
            mgr.hide();
        }
    }

    public static void keyboardUpdate(int sessionId, String value) {
        KeyboardManager mgr = sKeyboardManagers.get(sessionId);
        if (mgr != null) {
            mgr.updateValue(value);
        }
    }

    /**
     * Clean up Keyboard resources for a session. Call on session shutdown.
     */
    public static void destroyKeyboardManager(int sessionId) {
        KeyboardManager mgr = sKeyboardManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Bluetooth ====================

    /** Per-session Bluetooth managers. */
    private static final ConcurrentHashMap<Integer, BluetoothManager> sBluetoothManagers =
            new ConcurrentHashMap<>();

    private static BluetoothManager getOrCreateBluetoothManager(int sessionId) {
        BluetoothManager existing = sBluetoothManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sBluetoothLock) {
            existing = sBluetoothManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            BluetoothManager mgr = new BluetoothManager(sessionId, activity);
            sBluetoothManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void bluetoothOpenAdapter(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.openAdapter(optionsJson);
    }

    public static void bluetoothCloseAdapter(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.closeAdapter();
        }
    }

    public static String bluetoothGetAdapterState(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getAdapterState();
        }
        return "{\"discovering\":false,\"available\":false}";
    }

    public static void bluetoothStartDevicesDiscovery(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.startDiscovery(optionsJson);
    }

    public static void bluetoothStopDevicesDiscovery(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopDiscovery();
        }
    }

    public static String bluetoothGetDevices(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getDevices();
        }
        return "{\"devices\":[]}";
    }

    public static String bluetoothGetConnectedDevices(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getConnectedDevices(optionsJson);
        }
        return "{\"devices\":[]}";
    }

    public static void bluetoothMakePair(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.makePair(optionsJson);
    }

    public static void bluetoothIsDevicePaired(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.isDevicePaired(optionsJson);
    }

    public static void bluetoothStartBeaconDiscovery(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.startBeaconDiscovery(optionsJson);
    }

    public static void bluetoothStopBeaconDiscovery(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopBeaconDiscovery();
        }
    }

    public static String bluetoothGetBeacons(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getBeacons();
        }
        return "{\"beacons\":[]}";
    }

    // ---- BLE GATT ----

    public static void bleCreateConnection(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("createBLEConnection:fail no bluetooth manager");
        mgr.createBLEConnection(optionsJson);
    }

    public static void bleCloseConnection(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("closeBLEConnection:fail no bluetooth manager");
        mgr.closeBLEConnection(optionsJson);
    }

    public static String bleGetDeviceServices(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceServices:fail no bluetooth manager");
        return mgr.getBLEDeviceServices(optionsJson);
    }

    public static String bleGetDeviceCharacteristics(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceCharacteristics:fail no bluetooth manager");
        return mgr.getBLEDeviceCharacteristics(optionsJson);
    }

    public static void bleReadCharacteristicValue(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("readBLECharacteristicValue:fail no bluetooth manager");
        mgr.readBLECharacteristicValue(optionsJson);
    }

    public static void bleWriteCharacteristicValue(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("writeBLECharacteristicValue:fail no bluetooth manager");
        mgr.writeBLECharacteristicValue(optionsJson);
    }

    public static void bleNotifyCharacteristicValueChange(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("notifyBLECharacteristicValueChange:fail no bluetooth manager");
        mgr.notifyBLECharacteristicValueChange(optionsJson);
    }

    public static String bleGetDeviceRSSI(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceRSSI:fail no bluetooth manager");
        return mgr.getBLEDeviceRSSI(optionsJson);
    }

    public static void bleSetMTU(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("setBLEMTU:fail no bluetooth manager");
        mgr.setBLEMTU(optionsJson);
    }

    public static String bleGetMTU(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEMTU:fail no bluetooth manager");
        return mgr.getBLEMTU(optionsJson);
    }

    /**
     * Clean up Bluetooth resources for a session. Call on session shutdown.
     */
    public static void destroyBluetoothManager(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Image API ====================

    /** Per-session Image API managers. */
    private static final ConcurrentHashMap<Integer, ImageApiManager> sImageApiManagers =
            new ConcurrentHashMap<>();

    private static ImageApiManager getOrCreateImageApiManager(int sessionId) {
        ImageApiManager existing = sImageApiManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sImageApiLock) {
            existing = sImageApiManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            ImageApiManager mgr = new ImageApiManager(sessionId, activity);
            sImageApiManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void imageSaveToPhotosAlbum(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.saveToPhotosAlbum(optionsJson);
    }

    public static void imagePreviewMedia(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.previewMedia(optionsJson);
    }

    public static void imagePreviewImage(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.previewImage(optionsJson);
    }

    public static void imageCompress(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) {
            NativeMethods.onCompressImageResult(sessionId,
                    "{\"error\":\"compressImage:fail no context\"}");
            return;
        }
        mgr.compressAsync(sessionId, optionsJson);
    }

    public static void imageChooseMessageFile(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.chooseMessageFile(optionsJson);
    }

    public static void imageChooseImage(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;  // no context for session
        mgr.chooseImage(optionsJson);
    }

    /**
     * Clean up Image API resources for a session. Call on session shutdown.
     */
    public static void destroyImageApiManager(int sessionId) {
        ImageApiManager mgr = sImageApiManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Location ====================

    /**
     * Start an async location request (getLocation).
     * Result is delivered via {@link NativeMethods#onLocationResult}.
     * Called from native code via JNI.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with: type, altitude, isHighAccuracy, highAccuracyExpireTime
     */
    public static void getLocation(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            NativeMethods.onLocationResult(sessionId,
                    "{\"error\":\"getLocation:fail invalid session\"}");
            return;
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            NativeMethods.onLocationResult(sessionId,
                    "{\"error\":\"getLocation:fail no activity\"}");
            return;
        }
        LocationProvider.getLocationAsync(activity, sessionId, optionsJson);
    }

    /**
     * Start an async fuzzy location request (getFuzzyLocation).
     * Result is delivered via {@link NativeMethods#onFuzzyLocationResult}.
     * Called from native code via JNI.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with: type
     */
    public static void getFuzzyLocation(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            NativeMethods.onFuzzyLocationResult(sessionId,
                    "{\"error\":\"getFuzzyLocation:fail invalid session\"}");
            return;
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            NativeMethods.onFuzzyLocationResult(sessionId,
                    "{\"error\":\"getFuzzyLocation:fail no activity\"}");
            return;
        }
        LocationProvider.getFuzzyLocationAsync(activity, sessionId, optionsJson);
    }

    // ==================== Scan Code ====================

    /** Per-session Scan Code managers. */
    private static final ConcurrentHashMap<Integer, ScanCodeManager> sScanCodeManagers =
            new ConcurrentHashMap<>();

    private static ScanCodeManager getOrCreateScanCodeManager(int sessionId) {
        ScanCodeManager existing = sScanCodeManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sScanCodeLock) {
            existing = sScanCodeManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            ScanCodeManager mgr = new ScanCodeManager(sessionId, activity);
            sScanCodeManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void scanCode(int sessionId, String optionsJson) {
        ScanCodeManager mgr = getOrCreateScanCodeManager(sessionId);
        if (mgr == null) {
            NativeMethods.onScanCodeResult(sessionId,
                    "{\"error\":\"scanCode:fail no context\"}");
            return;
        }
        mgr.scanCode(optionsJson);
    }

    public static void destroyScanCodeManager(int sessionId) {
        ScanCodeManager mgr = sScanCodeManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Video ====================

    /** Per-session Video managers. */
    private static final ConcurrentHashMap<Integer, VideoManager> sVideoManagers =
            new ConcurrentHashMap<>();

    private static VideoManager getOrCreateVideoManager(int sessionId) {
        VideoManager existing = sVideoManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sVideoLock) {
            existing = sVideoManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            VideoManager mgr = new VideoManager(sessionId, activity);
            sVideoManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static String videoCreate(int sessionId, String optionsJson) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) throw new RuntimeException("videoCreate:fail no context");
        return mgr.create(optionsJson);
    }

    public static void videoPlay(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.play(videoId);
    }

    public static void videoPause(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.pause(videoId);
    }

    public static void videoStop(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.stop(videoId);
    }

    public static void videoSeek(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.seek(json);
    }

    public static void videoRequestFullscreen(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.requestFullscreen(json);
    }

    public static void videoExitFullscreen(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.exitFullscreen(videoId);
    }

    public static void videoSetProperty(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.setProperty(json);
    }

    public static void videoDestroy(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.destroyVideo(videoId);
    }

    /**
     * Clean up Video resources for a session. Call on session shutdown.
     */
    public static void destroyVideoManager(int sessionId) {
        VideoManager mgr = sVideoManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Game Log ====================

    private static final String GAME_LOG_TAG = "MigoGameLog";

    /**
     * Set or clear the game log handler for a session.
     *
     * @hide Called by {@link com.migo.runtime.GameSession#setGameLogHandler(GameLogHandler)}.
     */
    public static void setGameLogHandler(int sessionId, GameLogHandler handler) {
        if (handler == null) {
            sGameLogHandlers.remove(sessionId);
        } else {
            sGameLogHandlers.put(sessionId, handler);
        }
    }

    public static void gameLogReport(int sessionId, String logJson) {
        GameLogHandler h = sGameLogHandlers.get(sessionId);
        if (h != null) {
            h.onLog(logJson);
        } else {
            android.util.Log.d(GAME_LOG_TAG, "[session=" + sessionId + "] " + logJson);
        }
    }

    private static void clearGameLogHandler(int sessionId) {
        sGameLogHandlers.remove(sessionId);
    }

    // ==================== Auth ====================

    /**
     * Set or clear auth handler for a session.
     *
     * @hide Called by {@link com.migo.runtime.GameSession#setAuthHandler(AuthHandler)}.
     */
    public static void setAuthHandler(int sessionId, AuthHandler handler) {
        if (handler == null) {
            sAuthHandlers.remove(sessionId);
        } else {
            sAuthHandlers.put(sessionId, handler);
        }
    }

    private static void clearAuthHandler(int sessionId) {
        sAuthHandlers.remove(sessionId);
    }

    private static JSONObject parseAuthOptions(String optionsJson) {
        if (optionsJson == null || optionsJson.isEmpty()) {
            return new JSONObject();
        }
        try {
            return new JSONObject(optionsJson);
        } catch (Exception ignored) {
            return new JSONObject();
        }
    }

    private static int parseAuthRequestId(JSONObject options) {
        return options != null ? options.optInt("requestId", 0) : 0;
    }

    private static int parseAuthTimeout(JSONObject options) {
        return options != null ? options.optInt("timeout", 0) : 0;
    }

    private static boolean parseAuthBoolean(JSONObject options, String key, boolean defaultValue) {
        return options != null ? options.optBoolean(key, defaultValue) : defaultValue;
    }

    private static String parseAuthLang(JSONObject options) {
        String lang = options != null ? options.optString("lang", "en") : "en";
        if ("zh_CN".equals(lang) || "zh_TW".equals(lang) || "en".equals(lang)) {
            return lang;
        }
        return "en";
    }

    private static String normalizeAuthError(String reason) {
        if (reason == null) {
            return "unknown error";
        }
        String trimmed = reason.trim();
        return trimmed.isEmpty() ? "unknown error" : trimmed;
    }

    private static void reportLoginSuccess(int sessionId, int requestId, String code) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("code", code);
            NativeMethods.onLoginResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onLoginResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportLoginFail(int sessionId, int requestId, String reason) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("error", normalizeAuthError(reason));
            NativeMethods.onLoginResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onLoginResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportCheckSessionSuccess(int sessionId, int requestId) {
        NativeMethods.onCheckSessionResult(sessionId,
                "{\"requestId\":" + requestId + "}");
    }

    private static void reportCheckSessionFail(int sessionId, int requestId, String reason) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("error", normalizeAuthError(reason));
            NativeMethods.onCheckSessionResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onCheckSessionResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportGetUserInfoSuccess(
            int sessionId,
            int requestId,
            AuthHandler.UserInfoResult payload,
            boolean withCredentials,
            String lang
    ) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);

            AuthHandler.UserInfoResult source = payload != null ? payload : new AuthHandler.UserInfoResult();
            AuthHandler.UserInfo userInfo = source.userInfo != null ? source.userInfo : new AuthHandler.UserInfo();

            JSONObject user = new JSONObject();
            user.put("nickName", userInfo.nickName != null ? userInfo.nickName : "");
            user.put("avatarUrl", userInfo.avatarUrl != null ? userInfo.avatarUrl : "");
            user.put("gender", userInfo.gender);
            user.put("country", userInfo.country != null ? userInfo.country : "");
            user.put("province", userInfo.province != null ? userInfo.province : "");
            user.put("city", userInfo.city != null ? userInfo.city : "");
            user.put("language", userInfo.language != null ? userInfo.language : lang);
            res.put("userInfo", user);

            if (source.rawData != null && !source.rawData.isEmpty()) {
                res.put("rawData", source.rawData);
            }
            if (source.signature != null && !source.signature.isEmpty()) {
                res.put("signature", source.signature);
            }
            if (withCredentials) {
                if (source.encryptedData != null && !source.encryptedData.isEmpty()) {
                    res.put("encryptedData", source.encryptedData);
                }
                if (source.iv != null && !source.iv.isEmpty()) {
                    res.put("iv", source.iv);
                }
            }
            if (source.cloudID != null && !source.cloudID.isEmpty()) {
                res.put("cloudID", source.cloudID);
            }

            NativeMethods.onGetUserInfoResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onGetUserInfoResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportGetUserInfoFail(int sessionId, int requestId, String reason) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("error", normalizeAuthError(reason));
            NativeMethods.onGetUserInfoResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onGetUserInfoResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportGetPhoneNumberSuccess(int sessionId, int requestId, String code) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("code", code);
            NativeMethods.onGetPhoneNumberResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onGetPhoneNumberResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    private static void reportGetPhoneNumberFail(int sessionId, int requestId, String reason, Integer errno) {
        try {
            JSONObject res = new JSONObject();
            res.put("requestId", requestId);
            res.put("error", normalizeAuthError(reason));
            if (errno != null) {
                res.put("errno", errno.intValue());
            }
            NativeMethods.onGetPhoneNumberResult(sessionId, res.toString());
        } catch (JSONException ignored) {
            NativeMethods.onGetPhoneNumberResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"internal error\"}");
        }
    }

    /**
     * Trigger host-side login.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON: {"requestId":N,"timeout":ms}
     */
    public static void authLogin(int sessionId, String optionsJson) {
        JSONObject options = parseAuthOptions(optionsJson);
        int requestId = parseAuthRequestId(options);
        int timeout = parseAuthTimeout(options);

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            clearAuthHandler(sessionId);
            reportLoginFail(sessionId, requestId, "invalid session");
            return;
        }

        AuthHandler handler = sAuthHandlers.get(sessionId);
        if (handler == null) {
            reportLoginFail(sessionId, requestId, "no auth handler");
            return;
        }

        final java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try {
            handler.login(timeout, new AuthHandler.LoginCallback() {
                @Override
                public void onSuccess(String code) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    if (code == null || code.isEmpty()) {
                        reportLoginFail(sessionId, requestId, "invalid code");
                        return;
                    }
                    reportLoginSuccess(sessionId, requestId, code);
                }

                @Override
                public void onFailure(String reason) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportLoginFail(sessionId, requestId, reason);
                }
            });
        } catch (Exception e) {
            if (done.compareAndSet(false, true)) {
                reportLoginFail(sessionId, requestId,
                        e.getMessage() != null ? e.getMessage() : "unknown error");
            }
        }
    }

    /**
     * Trigger host-side checkSession.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON: {"requestId":N}
     */
    public static void authCheckSession(int sessionId, String optionsJson) {
        JSONObject options = parseAuthOptions(optionsJson);
        int requestId = parseAuthRequestId(options);

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            clearAuthHandler(sessionId);
            reportCheckSessionFail(sessionId, requestId, "invalid session");
            return;
        }

        AuthHandler handler = sAuthHandlers.get(sessionId);
        if (handler == null) {
            reportCheckSessionFail(sessionId, requestId, "no auth handler");
            return;
        }

        final java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try {
            handler.checkSession(new AuthHandler.CheckSessionCallback() {
                @Override
                public void onSuccess() {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportCheckSessionSuccess(sessionId, requestId);
                }

                @Override
                public void onFailure(String reason) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportCheckSessionFail(sessionId, requestId, reason);
                }
            });
        } catch (Exception e) {
            if (done.compareAndSet(false, true)) {
                reportCheckSessionFail(sessionId, requestId,
                        e.getMessage() != null ? e.getMessage() : "unknown error");
            }
        }
    }

    /**
     * Trigger host-side getUserInfo.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON: {"requestId":N,"withCredentials":bool,"lang":"en|zh_CN|zh_TW"}
     */
    public static void authGetUserInfo(int sessionId, String optionsJson) {
        JSONObject options = parseAuthOptions(optionsJson);
        int requestId = parseAuthRequestId(options);
        boolean withCredentials = parseAuthBoolean(options, "withCredentials", false);
        String lang = parseAuthLang(options);

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            clearAuthHandler(sessionId);
            reportGetUserInfoFail(sessionId, requestId, "invalid session");
            return;
        }

        AuthHandler handler = sAuthHandlers.get(sessionId);
        if (handler == null) {
            reportGetUserInfoFail(sessionId, requestId, "no auth handler");
            return;
        }

        final java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try {
            handler.getUserInfo(withCredentials, lang, new AuthHandler.UserInfoCallback() {
                @Override
                public void onSuccess(AuthHandler.UserInfoResult result) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportGetUserInfoSuccess(sessionId, requestId, result, withCredentials, lang);
                }

                @Override
                public void onFailure(String reason) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportGetUserInfoFail(sessionId, requestId, reason);
                }
            });
        } catch (Exception e) {
            if (done.compareAndSet(false, true)) {
                reportGetUserInfoFail(sessionId, requestId,
                        e.getMessage() != null ? e.getMessage() : "unknown error");
            }
        }
    }

    /**
     * Trigger host-side getPhoneNumber.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON: {"requestId":N,"isRealtime":bool,"phoneNumberNoQuotaToast":bool}
     */
    public static void authGetPhoneNumber(int sessionId, String optionsJson) {
        JSONObject options = parseAuthOptions(optionsJson);
        int requestId = parseAuthRequestId(options);
        boolean isRealtime = parseAuthBoolean(options, "isRealtime", false);
        boolean phoneNumberNoQuotaToast = parseAuthBoolean(options, "phoneNumberNoQuotaToast", true);

        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            clearAuthHandler(sessionId);
            reportGetPhoneNumberFail(sessionId, requestId, "invalid session", null);
            return;
        }

        AuthHandler handler = sAuthHandlers.get(sessionId);
        if (handler == null) {
            reportGetPhoneNumberFail(sessionId, requestId, "no auth handler", null);
            return;
        }

        final java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try {
            handler.getPhoneNumber(isRealtime, phoneNumberNoQuotaToast, new AuthHandler.PhoneNumberCallback() {
                @Override
                public void onSuccess(String code) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    if (code == null || code.isEmpty()) {
                        reportGetPhoneNumberFail(sessionId, requestId, "invalid code", null);
                        return;
                    }
                    reportGetPhoneNumberSuccess(sessionId, requestId, code);
                }

                @Override
                public void onFailure(String reason, Integer errno) {
                    if (!done.compareAndSet(false, true)) {
                        return;
                    }
                    reportGetPhoneNumberFail(sessionId, requestId, reason, errno);
                }
            });
        } catch (Exception e) {
            if (done.compareAndSet(false, true)) {
                reportGetPhoneNumberFail(sessionId, requestId,
                        e.getMessage() != null ? e.getMessage() : "unknown error", null);
            }
        }
    }

    // ==================== Subpackage ====================

    /**
     * Set or clear the subpackage download handler for a session.
     *
     * @hide Called by {@link com.migo.runtime.GameSession#setSubpackageHandler(SubpackageHandler)}.
     */
    public static void setSubpackageHandler(int sessionId, SubpackageHandler handler) {
        if (handler == null) {
            sSubpackageHandlers.remove(sessionId);
        } else {
            sSubpackageHandlers.put(sessionId, handler);
        }
    }

    private static void clearSubpackageHandler(int sessionId) {
        sSubpackageHandlers.remove(sessionId);
    }

    /**
     * Trigger a subpackage download.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON: {"requestId":N,"name":"stage1","root":"subpackages/stage1"}
     */
    public static void subpackageDownload(int sessionId, String optionsJson) {
        JSONObject options = parseAuthOptions(optionsJson);
        int requestId = parseAuthRequestId(options);
        String name = options.optString("name", "");
        String root = options.optString("root", "");

        SubpackageHandler handler = sSubpackageHandlers.get(sessionId);
        if (handler == null) {
            NativeMethods.onSubpackageResult(sessionId,
                    "{\"requestId\":" + requestId + ",\"error\":\"no subpackage handler\"}");
            return;
        }

        final java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try {
            handler.download(
                    new SubpackageHandler.SubpackageRequest(name, root),
                    new SubpackageHandler.DownloadCallback() {
                        @Override
                        public void onProgress(int progress, long totalBytesWritten, long totalBytesExpectedToWrite) {
                            try {
                                JSONObject res = new JSONObject();
                                res.put("requestId", requestId);
                                res.put("progress", progress);
                                res.put("totalBytesWritten", totalBytesWritten);
                                res.put("totalBytesExpectedToWrite", totalBytesExpectedToWrite);
                                NativeMethods.onSubpackageProgress(sessionId, res.toString());
                            } catch (JSONException ignored) {
                            }
                        }

                        @Override
                        public void onSuccess() {
                            if (!done.compareAndSet(false, true)) return;
                            NativeMethods.onSubpackageResult(sessionId,
                                    "{\"requestId\":" + requestId + "}");
                        }

                        @Override
                        public void onFailure(String reason) {
                            if (!done.compareAndSet(false, true)) return;
                            try {
                                JSONObject res = new JSONObject();
                                res.put("requestId", requestId);
                                res.put("error", reason != null ? reason : "download failed");
                                NativeMethods.onSubpackageResult(sessionId, res.toString());
                            } catch (JSONException ignored) {
                                NativeMethods.onSubpackageResult(sessionId,
                                        "{\"requestId\":" + requestId + ",\"error\":\"download failed\"}");
                            }
                        }
                    });
        } catch (Exception e) {
            if (done.compareAndSet(false, true)) {
                NativeMethods.onSubpackageResult(sessionId,
                        "{\"requestId\":" + requestId + ",\"error\":\""
                                + (e.getMessage() != null ? e.getMessage() : "unknown error") + "\"}");
            }
        }
    }

    // ==================== Session Cleanup ====================

    // ==================== Setting ====================

    /**
     * Open the mini program setting page.
     * The host should show a settings UI and call back via
     * {@link NativeMethods#onOpenSettingResult(int, String)}.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON options
     */
    public static void openSetting(int sessionId, String optionsJson) {
        // TODO: implement with a SettingManager when ready
        String rid = extractRequestId(optionsJson);
        NativeMethods.onOpenSettingResult(sessionId,
                buildDeferredErrorResult(rid, "openSetting:fail not supported", -2));
    }

    // ==================== Share ====================

    /**
     * Trigger the native share flow.
     * The host should show a share UI and call back via
     * {@link NativeMethods#onShareAppMessageResult(int, String)}.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with title, imageUrl, query
     */
    public static void shareAppMessage(int sessionId, String optionsJson) {
        // TODO: implement with a ShareManager when ready
        String rid = extractRequestId(optionsJson);
        NativeMethods.onShareAppMessageResult(sessionId,
                buildDeferredErrorResult(rid, "shareAppMessage:fail not supported", -2));
    }

    // ==================== Navigate ====================

    /**
     * Navigate to another mini program.
     * The host should perform navigation and call back via
     * {@link NativeMethods#onNavigateToMiniProgramResult(int, String)}.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with appId, path, extraData, envVersion
     */
    public static void navigateToMiniProgram(int sessionId, String optionsJson) {
        // TODO: implement navigation when ready
        String rid = extractRequestId(optionsJson);
        NativeMethods.onNavigateToMiniProgramResult(sessionId,
                buildDeferredErrorResult(rid, "navigateToMiniProgram:fail not supported", -2));
    }

    /**
     * Open the customer service conversation.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with sessionFrom, showMessageCard, etc.
     */
    public static void openCustomerServiceConversation(int sessionId, String optionsJson) {
        // TODO: implement customer service when ready
        throw new RuntimeException("openCustomerServiceConversation:fail not supported");
    }

    // ==================== Payment ====================

    /**
     * Check if the current environment supports Midas payment.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON options
     * @return JSON string: {"data":{"allow_pay":true/false}}
     */
    public static String checkIsSupportMidasPayment(int sessionId, String optionsJson) {
        // TODO: implement real payment check when ready
        return "{\"data\":{\"allow_pay\":false}}";
    }

    /**
     * Trigger Midas payment flow.
     * The host should show payment UI and call back via
     * {@link NativeMethods#onMidasPaymentResult(int, String)}.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with mode, env, offerId, currencyType, etc.
     */
    public static void requestMidasPayment(int sessionId, String optionsJson) {
        // TODO: implement with a PaymentManager when ready
        String rid = extractRequestId(optionsJson);
        NativeMethods.onMidasPaymentResult(sessionId,
                buildDeferredErrorResult(rid, "requestMidasPayment:fail not supported", -2));
    }

    /**
     * Trigger Midas payment for game items.
     * The host should show payment UI and call back via
     * {@link NativeMethods#onMidasPaymentGameItemResult(int, String)}.
     *
     * @param sessionId   The session ID
     * @param optionsJson JSON with signData, paySig, signature
     */
    public static void requestMidasPaymentGameItem(int sessionId, String optionsJson) {
        // TODO: implement with a PaymentManager when ready
        String rid = extractRequestId(optionsJson);
        NativeMethods.onMidasPaymentGameItemResult(sessionId,
                buildDeferredErrorResult(rid, "requestMidasPaymentGameItem:fail not supported", -2));
    }

    /**
     * Destroy all per-session managers. Called from GameSession.close().
     * This is the single cleanup entry point to prevent resource leaks.
     *
     * @param sessionId The session ID
     */
    public static void destroyAllManagers(int sessionId) {
        destroySensorManager(sessionId);
        destroyNetworkMonitor(sessionId);
        destroyCaptureObserver(sessionId);
        destroyRecorderManager(sessionId);
        destroyCameraManagers(sessionId);
        destroyKeyboardManager(sessionId);
        destroyBluetoothManager(sessionId);
        destroyImageApiManager(sessionId);
        destroyScanCodeManager(sessionId);
        destroyVideoManager(sessionId);
        clearGameLogHandler(sessionId);
        clearAuthHandler(sessionId);
        clearSubpackageHandler(sessionId);
        unregisterErrorCallback(sessionId);
    }

}
