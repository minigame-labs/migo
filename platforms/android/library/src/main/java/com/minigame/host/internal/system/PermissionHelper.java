package com.minigame.host.internal.system;

import android.Manifest;
import android.app.NotificationManager;
import android.content.Context;
import android.content.pm.PackageManager;
import android.os.Build;
import android.util.Log;

public final class PermissionHelper {

    private PermissionHelper() {
    }

    private static final String AUTHORIZED = "authorized";
    private static final String DENIED = "denied";
    private static final String NOT_DETERMINED = "not determined";

    private static final String TAG = "PermissionHelper";

    public static String getAsJson(Context context) {
        Context ctx = context.getApplicationContext();

        Log.d(TAG, "getAsJson: building permissions JSON");

        StringBuilder sb = new StringBuilder(384);
        sb.append('{');

        append(sb, "albumAuthorized", getPermissionStatus(ctx, Manifest.permission.READ_EXTERNAL_STORAGE));
        append(sb, "bluetoothAuthorized", getBluetoothPermissionStatus(ctx));
        append(sb, "cameraAuthorized", getPermissionStatus(ctx, Manifest.permission.CAMERA));
        append(sb, "locationAuthorized", getLocationPermissionStatus(ctx));
        append(sb, "locationReducedAccuracy", isLocationReducedAccuracy(ctx));
        append(sb, "microphoneAuthorized", getPermissionStatus(ctx, Manifest.permission.RECORD_AUDIO));
        append(sb, "notificationAuthorized", getNotificationPermissionStatus(ctx));

        // placeholders for protocol compatibility
        append(sb, "notificationAlertAuthorized", NOT_DETERMINED);
        append(sb, "notificationBadgeAuthorized", NOT_DETERMINED);
        append(sb, "notificationSoundAuthorized", NOT_DETERMINED);

        append(sb, "phoneCalendarAuthorized", getCalendarPermissionStatus(ctx));

        // remove trailing comma
        sb.setLength(sb.length() - 1);
        sb.append('}');

        String json = sb.toString();
        Log.d(TAG, "getAsJson: result=" + json);
        return json;
    }

    // ------------------------------------------------------------------------
    // Permission helpers
    // ------------------------------------------------------------------------

    private static String getPermissionStatus(Context ctx, String permission) {
        try {
            int result;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                result = ctx.checkSelfPermission(permission);
            } else {
                result = ctx.getPackageManager().checkPermission(permission, ctx.getPackageName());
            }
            boolean granted = result == PackageManager.PERMISSION_GRANTED;
            Log.d(TAG, "getPermissionStatus: " + permission + " -> " + (granted ? AUTHORIZED : DENIED));
            return granted ? AUTHORIZED : DENIED;
        } catch (Throwable ignored) {
            Log.d(TAG, "getPermissionStatus: exception checking " + permission, ignored);
            return NOT_DETERMINED;
        }
    }

    // ------------------------------------------------------------------------
    // Location
    // ------------------------------------------------------------------------

    private static String getLocationPermissionStatus(Context ctx) {
        if (AUTHORIZED.equals(getPermissionStatus(ctx, Manifest.permission.ACCESS_FINE_LOCATION))) {
            Log.d(TAG, "getLocationPermissionStatus: fine location authorized");
            return AUTHORIZED;
        }
        String coarse = getPermissionStatus(ctx, Manifest.permission.ACCESS_COARSE_LOCATION);
        Log.d(TAG, "getLocationPermissionStatus: coarse -> " + coarse);
        return coarse;
    }

    private static boolean isLocationReducedAccuracy(Context ctx) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            boolean fine = ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED;
            boolean coarse = ctx.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED;
            boolean reduced = coarse && !fine;
            Log.d(TAG, "isLocationReducedAccuracy: fine=" + fine + " coarse=" + coarse + " reduced=" + reduced);
            return reduced;
        }
        Log.d(TAG, "isLocationReducedAccuracy: API<31 -> false");
        return false;
    }

    // ------------------------------------------------------------------------
    // Bluetooth
    // ------------------------------------------------------------------------

    private static String getBluetoothPermissionStatus(Context ctx) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            String res = getPermissionStatus(ctx, Manifest.permission.BLUETOOTH_CONNECT);
            Log.d(TAG, "getBluetoothPermissionStatus (API>=S): " + res);
            return res;
        }
        Log.d(TAG, "getBluetoothPermissionStatus: API<S -> assumed authorized");
        return AUTHORIZED;
    }

    // ------------------------------------------------------------------------
    // Notification
    // ------------------------------------------------------------------------

    private static String getNotificationPermissionStatus(Context ctx) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            String res = getPermissionStatus(ctx, Manifest.permission.POST_NOTIFICATIONS);
            Log.d(TAG, "getNotificationPermissionStatus (API>=T): " + res);
            return res;
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            NotificationManager nm = (NotificationManager) ctx.getSystemService(Context.NOTIFICATION_SERVICE);
            if (nm != null) {
                boolean enabled = nm.areNotificationsEnabled();
                Log.d(TAG, "getNotificationPermissionStatus (API>=N): enabled=" + enabled);
                return enabled ? AUTHORIZED : DENIED;
            } else {
                Log.d(TAG, "getNotificationPermissionStatus: NotificationManager null");
            }
        }

        Log.d(TAG, "getNotificationPermissionStatus: default -> authorized");
        return AUTHORIZED;
    }

    // ------------------------------------------------------------------------
    // Calendar
    // ------------------------------------------------------------------------

    private static String getCalendarPermissionStatus(Context ctx) {
        boolean read = AUTHORIZED.equals(getPermissionStatus(ctx, Manifest.permission.READ_CALENDAR));
        boolean write = AUTHORIZED.equals(getPermissionStatus(ctx, Manifest.permission.WRITE_CALENDAR));
        String res = (read && write) ? AUTHORIZED : DENIED;
        Log.d(TAG, "getCalendarPermissionStatus: read=" + read + " write=" + write + " -> " + res);
        return res;
    }

    // ------------------------------------------------------------------------
    // JSON helpers
    // ------------------------------------------------------------------------

    private static void append(StringBuilder sb, String key, String value) {
        sb.append('"').append(key).append("\":\"").append(value).append("\",");
    }

    private static void append(StringBuilder sb, String key, boolean value) {
        sb.append('"').append(key).append("\":").append(value).append(',');
    }
}
