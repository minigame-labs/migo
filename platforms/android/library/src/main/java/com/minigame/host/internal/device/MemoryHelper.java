package com.minigame.host.internal.device;

import android.app.ActivityManager;
import android.content.Context;
import android.os.Build;
import android.util.Log;

public final class MemoryHelper {

    private static final long DEFAULT_TOTAL_MB = 1024;
    private static final String TAG = "MemoryHelper";

    private MemoryHelper() {
    }

    public static long getTotalMemoryMB(Context ctx) {
        Log.d(TAG, "getTotalMemoryMB: enter");
        if (ctx == null) {
            Log.d(TAG, "getTotalMemoryMB: ctx is null, returning default " + DEFAULT_TOTAL_MB);
            return DEFAULT_TOTAL_MB;
        }

        try {
            Context app = ctx.getApplicationContext();
            ActivityManager am = (ActivityManager) app.getSystemService(Context.ACTIVITY_SERVICE);
            if (am == null) {
                Log.d(TAG, "getTotalMemoryMB: ActivityManager is null, returning default " + DEFAULT_TOTAL_MB);
                return DEFAULT_TOTAL_MB;
            }

            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.JELLY_BEAN) {
                Log.d(TAG, "getTotalMemoryMB: API < JELLY_BEAN, returning default " + DEFAULT_TOTAL_MB);
                return DEFAULT_TOTAL_MB;
            }

            ActivityManager.MemoryInfo mi = new ActivityManager.MemoryInfo();
            am.getMemoryInfo(mi);

            long totalBytes = mi.totalMem;
            if (totalBytes <= 0) {
                Log.d(TAG, "getTotalMemoryMB: totalMem <= 0, returning default " + DEFAULT_TOTAL_MB);
                return DEFAULT_TOTAL_MB;
            }

            long mb = totalBytes / (1024L * 1024L);
            Log.d(TAG, "getTotalMemoryMB: totalMemBytes=" + totalBytes + " totalMB=" + mb);
            return mb;
        } catch (Throwable t) {
            Log.d(TAG, "getTotalMemoryMB: exception, returning default " + DEFAULT_TOTAL_MB, t);
            return DEFAULT_TOTAL_MB;
        }
    }
}
