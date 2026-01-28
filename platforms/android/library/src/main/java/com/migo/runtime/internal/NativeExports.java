package com.migo.runtime.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.provider.Settings;

import com.migo.runtime.internal.platform.DeviceInfo;
import com.migo.runtime.internal.platform.DisplayCompat;
import com.migo.runtime.internal.platform.Permissions;
import com.migo.runtime.internal.platform.SystemSettings;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

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

    // ==================== Permissions ====================

    /**
     * Get app authorization settings as JSON string.
     *
     * @return JSON string with permission states
     */
    public static String getAppAuthorizationSettingJson() {
        RuntimeContext context = RuntimeRegistry.getAny();
        Activity activity = context != null ? context.getActivity() : null;
        return Permissions.toJson(activity);
    }
}
