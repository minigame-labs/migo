package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.content.Context;
import android.content.pm.PackageManager;
import android.os.Build;

/**
 * Permission utilities.
 * <p>
 * Compatible with Android API 21+.
 *
 * @hide
 */
public final class Permissions {

    private Permissions() {}

    // Common permission strings
    public static final String CAMERA = "android.permission.CAMERA";
    public static final String RECORD_AUDIO = "android.permission.RECORD_AUDIO";
    public static final String FINE_LOCATION = "android.permission.ACCESS_FINE_LOCATION";
    public static final String COARSE_LOCATION = "android.permission.ACCESS_COARSE_LOCATION";
    public static final String WRITE_STORAGE = "android.permission.WRITE_EXTERNAL_STORAGE";
    public static final String READ_STORAGE = "android.permission.READ_EXTERNAL_STORAGE";
    public static final String BLUETOOTH = "android.permission.BLUETOOTH";
    public static final String BLUETOOTH_ADMIN = "android.permission.BLUETOOTH_ADMIN";
    public static final String BLUETOOTH_CONNECT = "android.permission.BLUETOOTH_CONNECT";
    public static final String BLUETOOTH_SCAN = "android.permission.BLUETOOTH_SCAN";

    /**
     * Permission state.
     */
    public enum State {
        /** Permission granted */
        AUTHORIZED("authorized"),
        /** Permission denied */
        DENIED("denied"),
        /** Permission not determined (not requested yet) */
        NOT_DETERMINED("not determined");

        private final String value;

        State(String value) {
            this.value = value;
        }

        public String getValue() {
            return value;
        }
    }

    /**
     * Check if a permission is granted.
     *
     * @param context    The context
     * @param permission The permission to check
     * @return true if granted
     */
    public static boolean isGranted(Context context, String permission) {
        if (context == null || permission == null) return false;
        
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            return context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
        } else {
            // Pre-M: permissions are granted at install time
            return true;
        }
    }

    /**
     * Get permission state.
     *
     * @param activity   The activity
     * @param permission The permission to check
     * @return Permission state
     */
    public static State getState(Activity activity, String permission) {
        if (activity == null || permission == null) {
            return State.NOT_DETERMINED;
        }
        
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            int result = activity.checkSelfPermission(permission);
            if (result == PackageManager.PERMISSION_GRANTED) {
                return State.AUTHORIZED;
            } else {
                // Could be denied or not determined
                // If shouldShowRequestPermissionRationale is true, it was denied
                if (activity.shouldShowRequestPermissionRationale(permission)) {
                    return State.DENIED;
                }
                return State.NOT_DETERMINED;
            }
        } else {
            // Pre-M: always authorized
            return State.AUTHORIZED;
        }
    }

    /**
     * Check multiple permissions at once.
     *
     * @param context     The context
     * @param permissions The permissions to check
     * @return true if all granted
     */
    public static boolean areAllGranted(Context context, String... permissions) {
        if (context == null || permissions == null) return false;
        
        for (String permission : permissions) {
            if (!isGranted(context, permission)) {
                return false;
            }
        }
        return true;
    }

    /**
     * Get app authorization settings as JSON string.
     * <p>
     * Format matches the expected native protocol.
     *
     * @param activity The activity
     * @return JSON string with permission states
     */
    public static String toJson(Activity activity) {
        StringBuilder sb = new StringBuilder(256);
        sb.append("{");

        String[] permissions = { CAMERA, RECORD_AUDIO, FINE_LOCATION, WRITE_STORAGE, READ_STORAGE };
        String[] keys = { "camera", "record", "location", "writePhotosAlbum", "album" };

        for (int i = 0; i < permissions.length; i++) {
            if (i > 0) sb.append(",");
            
            State state = activity != null ? getState(activity, permissions[i]) : State.NOT_DETERMINED;
            sb.append("\"").append(keys[i]).append("\":\"").append(state.getValue()).append("\"");
        }

        sb.append("}");
        return sb.toString();
    }
}
