package com.migo.host.internal;

import android.app.Activity;
import android.bluetooth.BluetoothAdapter;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Rect;
import android.location.LocationManager;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.provider.Settings;
import android.util.DisplayMetrics;
import android.view.Display;
import android.view.Surface;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;

import com.migo.host.internal.jni.HostNative;
import com.migo.host.internal.runtime.HostRuntime;
import com.migo.host.internal.runtime.HostRuntimeRegistry;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Static methods exposed to native code via JNI.
 * <p>
 * These methods are called from Rust/native code to access Android platform features.
 * Method signatures must match those registered in registration.rs.
 */
public final class NativeExports {

    private static final int BLUETOOTH_SETTING_REQUEST_CODE = 10001;

    private NativeExports() {
        // Prevent instantiation
    }

    /**
     * Get the app's cache directory path.
     *
     * @return Cache directory path, or empty string if unavailable
     */
    public static String getCacheDirPath() {
        HostRuntime runtime = HostRuntimeRegistry.getAny();
        if (runtime == null) {
            return "";
        }
        Activity activity = runtime.getActivity();
        if (activity == null) {
            return "";
        }
        File cacheDir = activity.getCacheDir();
        return cacheDir != null ? cacheDir.getAbsolutePath() : "";
    }

    /**
     * Open system Bluetooth settings.
     *
     * @param hostId The host ID to receive the callback
     */
    public static void openSystemBluetoothSetting(int hostId) {
        HostRuntime runtime = HostRuntimeRegistry.get(hostId);
        if (runtime == null) {
            HostNative.onBluetoothSettingResult(hostId, false);
            return;
        }

        Activity activity = runtime.getActivity();
        if (activity == null) {
            HostNative.onBluetoothSettingResult(hostId, false);
            return;
        }

        try {
            Intent intent = new Intent(Settings.ACTION_BLUETOOTH_SETTINGS);
            activity.startActivityForResult(intent, BLUETOOTH_SETTING_REQUEST_CODE);
        } catch (Exception e) {
            HostNative.onBluetoothSettingResult(hostId, false);
        }
    }

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
     * @param hostId The host ID
     * @return Packed byte array, or null if unavailable
     */
    public static byte[] getWindowInfoBytes(int hostId) {
        HostRuntime runtime = HostRuntimeRegistry.get(hostId);
        if (runtime == null) {
            return null;
        }

        Activity activity = runtime.getActivity();
        if (activity == null) {
            return null;
        }

        try {
            Window window = activity.getWindow();
            View decorView = window.getDecorView();
            WindowManager wm = activity.getWindowManager();
            Display display = wm.getDefaultDisplay();
            DisplayMetrics metrics = new DisplayMetrics();
            display.getRealMetrics(metrics);

            int windowWidth = decorView.getWidth();
            int windowHeight = decorView.getHeight();
            int screenWidth = metrics.widthPixels;
            int screenHeight = metrics.heightPixels;
            float density = metrics.density;

            // Status bar height
            int statusBarHeight = 0;
            int resourceId = activity.getResources().getIdentifier("status_bar_height", "dimen", "android");
            if (resourceId > 0) {
                statusBarHeight = activity.getResources().getDimensionPixelSize(resourceId);
            }

            // Screen top (location of window on screen)
            int[] location = new int[2];
            decorView.getLocationOnScreen(location);
            int screenTop = location[1];

            // Safe area insets
            int safeLeft = 0, safeTop = 0, safeRight = 0, safeBottom = 0;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                WindowInsets insets = decorView.getRootWindowInsets();
                if (insets != null) {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                        android.graphics.Insets systemBars = insets.getInsets(
                                WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout()
                        );
                        safeLeft = systemBars.left;
                        safeTop = systemBars.top;
                        safeRight = systemBars.right;
                        safeBottom = systemBars.bottom;
                    } else {
                        android.view.DisplayCutout cutout = insets.getDisplayCutout();
                        if (cutout != null) {
                            safeLeft = cutout.getSafeInsetLeft();
                            safeTop = cutout.getSafeInsetTop();
                            safeRight = cutout.getSafeInsetRight();
                            safeBottom = cutout.getSafeInsetBottom();
                        }
                    }
                }
            }

            ByteBuffer buffer = ByteBuffer.allocate(52);
            buffer.order(ByteOrder.nativeOrder());
            buffer.putInt(windowWidth);     // 0-3
            buffer.putInt(windowHeight);    // 4-7
            buffer.putInt(screenWidth);     // 8-11
            buffer.putInt(screenHeight);    // 12-15
            buffer.putInt(statusBarHeight); // 16-19
            buffer.putInt((int) (density * 1000)); // 20-23: pixelRatio * 1000
            buffer.putInt(screenTop);       // 24-27
            buffer.putInt(0);               // 28-31: reserved
            buffer.putInt(0);               // 32-35: reserved
            buffer.putInt(safeLeft);        // 36-39
            buffer.putInt(safeTop);         // 40-43
            buffer.putInt(safeRight);       // 44-47
            buffer.putInt(safeBottom);      // 48-51

            return buffer.array();
        } catch (Exception e) {
            return null;
        }
    }

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
        HostRuntime runtime = HostRuntimeRegistry.getAny();
        Activity activity = runtime != null ? runtime.getActivity() : null;
        Context context = activity;

        byte[] result = new byte[4];

        // Bluetooth
        try {
            BluetoothAdapter adapter = BluetoothAdapter.getDefaultAdapter();
            result[0] = (byte) (adapter != null && adapter.isEnabled() ? 1 : 0);
        } catch (Exception e) {
            result[0] = 0;
        }

        // Location
        try {
            if (context != null) {
                LocationManager lm = (LocationManager) context.getSystemService(Context.LOCATION_SERVICE);
                result[1] = (byte) (lm != null && (lm.isProviderEnabled(LocationManager.GPS_PROVIDER)
                        || lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER)) ? 1 : 0);
            }
        } catch (Exception e) {
            result[1] = 0;
        }

        // WiFi
        try {
            if (context != null) {
                WifiManager wm = (WifiManager) context.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
                result[2] = (byte) (wm != null && wm.isWifiEnabled() ? 1 : 0);
            }
        } catch (Exception e) {
            result[2] = 0;
        }

        // Orientation
        if (activity != null) {
            int rotation = activity.getWindowManager().getDefaultDisplay().getRotation();
            // 0, 2 = portrait; 1, 3 = landscape
            result[3] = (byte) ((rotation == Surface.ROTATION_0 || rotation == Surface.ROTATION_180) ? 1 : 2);
        } else {
            result[3] = 0;
        }

        return result;
    }

    /**
     * Get device information as JSON string.
     *
     * @return JSON string with device info
     */
    public static String getDeviceInfoJson() {
        StringBuilder sb = new StringBuilder("{");

        sb.append("\"brand\":\"").append(escapeJson(Build.BRAND)).append("\",");
        sb.append("\"model\":\"").append(escapeJson(Build.MODEL)).append("\",");
        sb.append("\"system\":\"Android\",");
        sb.append("\"platform\":\"android\",");
        sb.append("\"version\":\"").append(Build.VERSION.RELEASE).append("\",");
        sb.append("\"SDKVersion\":").append(Build.VERSION.SDK_INT).append(",");
        sb.append("\"language\":\"").append(java.util.Locale.getDefault().getLanguage()).append("\",");
        sb.append("\"fontSizeSetting\":16,");
        sb.append("\"benchmarkLevel\":-1");

        sb.append("}");
        return sb.toString();
    }

    /**
     * Get app authorization settings as JSON string.
     *
     * @return JSON string with permission states
     */
    public static String getAppAuthorizationSettingJson() {
        HostRuntime runtime = HostRuntimeRegistry.getAny();
        Activity activity = runtime != null ? runtime.getActivity() : null;

        StringBuilder sb = new StringBuilder("{");

        // Check common permissions
        String[] permissions = {
                "android.permission.CAMERA",
                "android.permission.RECORD_AUDIO",
                "android.permission.ACCESS_FINE_LOCATION",
                "android.permission.WRITE_EXTERNAL_STORAGE",
                "android.permission.READ_EXTERNAL_STORAGE"
        };

        String[] keys = {"camera", "record", "location", "writePhotosAlbum", "album"};

        for (int i = 0; i < permissions.length; i++) {
            if (i > 0) sb.append(",");
            String state = "not determined";
            if (activity != null) {
                int result = activity.checkSelfPermission(permissions[i]);
                state = (result == PackageManager.PERMISSION_GRANTED) ? "authorized" : "denied";
            }
            sb.append("\"").append(keys[i]).append("\":\"").append(state).append("\"");
        }

        sb.append("}");
        return sb.toString();
    }

    /**
     * Escape special characters for JSON string.
     */
    private static String escapeJson(String s) {
        if (s == null) return "";
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
}
