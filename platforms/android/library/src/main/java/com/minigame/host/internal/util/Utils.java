package com.minigame.host.internal.util;

import android.app.Activity;
import android.content.Context;
import android.content.ContextWrapper;
import android.os.Build;
import android.util.DisplayMetrics;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import java.util.Objects;

public final class Utils {
    public static Activity getActivity(Context ctx) {
        if (ctx == null) return null;
        if (ctx instanceof Activity) return (Activity) ctx;
        if (ctx instanceof ContextWrapper) {
            Context base = ((ContextWrapper) ctx).getBaseContext();
            return getActivity(base);
        }
        return null;
    }

    public static float getDpi(Context ctx) {
        Objects.requireNonNull(ctx);

        try {
            DisplayMetrics dm = new DisplayMetrics();
            WindowManager wm = (WindowManager) ctx.getSystemService(Context.WINDOW_SERVICE);
            if (wm != null && wm.getDefaultDisplay() != null) {
                wm.getDefaultDisplay().getMetrics(dm);
                if (dm.density > 0f) return dm.density;
            }
        } catch (Throwable ignored) {
        }
        return 1.0f;
    }

    // little-endian int
    public static void putInt(byte[] array, int offset, int value) {
        array[offset] = (byte) value;
        array[offset + 1] = (byte) (value >> 8);
        array[offset + 2] = (byte) (value >> 16);
        array[offset + 3] = (byte) (value >> 24);
    }

    public static void enterFullScreen(Activity activity) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            return;
        }

        final Window window = activity.getWindow();

        window.addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN);

        final View decor = window.getDecorView();
        int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION;

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.KITKAT) { // API 19+
            flags |= View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_FULLSCREEN | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY;
        }

        decor.setSystemUiVisibility(flags);

        int finalFlags = flags;
        decor.setOnSystemUiVisibilityChangeListener(visibility -> {
            if ((visibility & View.SYSTEM_UI_FLAG_FULLSCREEN) == 0) {
                decor.setSystemUiVisibility(finalFlags);
            }
        });

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) { // API 28+
            WindowManager.LayoutParams lp = window.getAttributes();
            lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            window.setAttributes(lp);
        }
    }
}
