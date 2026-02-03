package com.migo.runtime.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.provider.Settings;

import com.migo.runtime.internal.platform.BatteryInfo;
import com.migo.runtime.internal.platform.DeviceInfo;
import com.migo.runtime.internal.platform.DeviceSensorManager;
import com.migo.runtime.internal.platform.InteractionUI;
import com.migo.runtime.internal.platform.DisplayCompat;
import com.migo.runtime.internal.platform.Permissions;
import com.migo.runtime.internal.platform.ScreenBrightness;
import com.migo.runtime.internal.platform.SystemSettings;
import com.migo.runtime.internal.platform.Vibrator;

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

    private NativeExports() {}

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
}
