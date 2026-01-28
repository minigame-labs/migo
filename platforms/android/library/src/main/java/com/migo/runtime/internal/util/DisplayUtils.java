package com.migo.runtime.internal.util;

import android.app.Activity;
import android.content.Context;
import android.content.ContextWrapper;
import android.util.DisplayMetrics;
import android.view.WindowManager;

/**
 * Display-related utility methods.
 *
 * @hide
 */
public final class DisplayUtils {

    private DisplayUtils() {}

    /**
     * Extract Activity from a Context.
     *
     * @param context The context
     * @return The Activity, or null if not found
     */
    public static Activity getActivity(Context context) {
        if (context == null) return null;
        if (context instanceof Activity) return (Activity) context;
        if (context instanceof ContextWrapper) {
            Context base = ((ContextWrapper) context).getBaseContext();
            return getActivity(base);
        }
        return null;
    }

    /**
     * Get display density from context.
     *
     * @param context The context
     * @return Display density (pixels per dp), defaults to 1.0 on error
     */
    public static float getDensity(Context context) {
        if (context == null) {
            return 1.0f;
        }

        try {
            DisplayMetrics dm = new DisplayMetrics();
            WindowManager wm = (WindowManager) context.getSystemService(Context.WINDOW_SERVICE);
            if (wm != null && wm.getDefaultDisplay() != null) {
                wm.getDefaultDisplay().getMetrics(dm);
                if (dm.density > 0f) return dm.density;
            }
        } catch (Throwable ignored) {
        }
        return 1.0f;
    }

    /**
     * Get screen width in pixels.
     *
     * @param context The context
     * @return Screen width, or 0 on error
     */
    public static int getScreenWidth(Context context) {
        if (context == null) return 0;
        try {
            DisplayMetrics dm = context.getResources().getDisplayMetrics();
            return dm.widthPixels;
        } catch (Throwable ignored) {
            return 0;
        }
    }

    /**
     * Get screen height in pixels.
     *
     * @param context The context
     * @return Screen height, or 0 on error
     */
    public static int getScreenHeight(Context context) {
        if (context == null) return 0;
        try {
            DisplayMetrics dm = context.getResources().getDisplayMetrics();
            return dm.heightPixels;
        } catch (Throwable ignored) {
            return 0;
        }
    }

    /**
     * Convert dp to pixels.
     *
     * @param context The context
     * @param dp      Value in dp
     * @return Value in pixels
     */
    public static int dpToPx(Context context, float dp) {
        return Math.round(dp * getDensity(context));
    }

    /**
     * Convert pixels to dp.
     *
     * @param context The context
     * @param px      Value in pixels
     * @return Value in dp
     */
    public static float pxToDp(Context context, int px) {
        return px / getDensity(context);
    }

    /**
     * Write an integer to a byte array in little-endian format.
     *
     * @param array  Target array
     * @param offset Starting offset
     * @param value  Integer value
     */
    public static void putIntLE(byte[] array, int offset, int value) {
        array[offset] = (byte) value;
        array[offset + 1] = (byte) (value >> 8);
        array[offset + 2] = (byte) (value >> 16);
        array[offset + 3] = (byte) (value >> 24);
    }
}
