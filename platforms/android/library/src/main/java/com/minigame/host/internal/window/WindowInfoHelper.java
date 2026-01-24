package com.minigame.host.internal.window;

import static com.minigame.host.internal.util.Utils.putInt;

import android.app.Activity;
import android.content.Context;
import android.content.res.Resources;
import android.graphics.Rect;
import android.os.Build;
import android.util.DisplayMetrics;
import android.util.Log;
import android.view.DisplayCutout;
import android.view.View;
import android.view.WindowInsets;

import com.minigame.host.internal.util.Utils;

public final class WindowInfoHelper {
    private static byte[] cached;
    private static long lastTime;
    private static final long CACHE_MS = 30000;
    private static final String TAG = "WindowInfoHelper";

    public static byte[] getAsBytes(Activity activity) {
        long now = System.currentTimeMillis();
        if (cached != null && (now - lastTime) < CACHE_MS) {
            Log.d(TAG, "getAsBytes: cache hit, returning cached bytes");
            return cached;
        }

        Log.d(TAG, "getAsBytes: cache miss, rebuilding WindowInfo");
        WindowInfo info = build(activity);
        lastTime = now;
        cached = serialize(info);
        Log.d(TAG, "getAsBytes: serialized WindowInfo size=" + cached.length);
        return cached;
    }

    private static byte[] serialize(WindowInfo info) {
        byte[] result = new byte[52]; // 13 * 4
        int pixelRatioInt = (int) (info.pixelRatio * 1000);
        int safeAreaLeft = info.safeArea != null ? info.safeArea.left : 0;
        int safeAreaTop = info.safeArea != null ? info.safeArea.top : 0;
        int safeAreaRight = info.safeArea != null ? info.safeArea.right : 0;
        int safeAreaBottomVal = info.safeArea != null ? info.safeArea.bottom : 0;

        putInt(result, 0, info.windowWidth);
        putInt(result, 4, info.windowHeight);
        putInt(result, 8, info.screenWidth);
        putInt(result, 12, info.screenHeight);
        putInt(result, 16, info.statusBarHeight);
        putInt(result, 20, pixelRatioInt);
        putInt(result, 24, info.windowTop);
        putInt(result, 28, info.windowBottom);
        putInt(result, 32, info.safeAreaBottom);
        putInt(result, 36, safeAreaLeft);
        putInt(result, 40, safeAreaTop);
        putInt(result, 44, safeAreaRight);
        putInt(result, 48, safeAreaBottomVal);

        Log.d(TAG, String.format("serialize: window %dx%d screen %dx%d statusBar=%d pixelRatio=%d safeArea=[%d,%d,%d,%d] safeAreaBottom=%d", info.windowWidth, info.windowHeight, info.screenWidth, info.screenHeight, info.statusBarHeight, pixelRatioInt, safeAreaLeft, safeAreaTop, safeAreaRight, safeAreaBottomVal, info.safeAreaBottom));

        return result;
    }

    private static WindowInfo build(Activity activity) {
        WindowInfo info = new WindowInfo();

        if (activity == null) {
            Log.d(TAG, "build: activity is null, returning default WindowInfo");
            return info;
        }

        View decor = activity.getWindow().getDecorView();
        Resources res = activity.getResources();

        //------------------------------------------------------------------
        // 1. Get real screen dimensions
        //------------------------------------------------------------------
        DisplayMetrics dm = new DisplayMetrics();
        activity.getWindowManager().getDefaultDisplay().getRealMetrics(dm);

        info.screenWidth = dm.widthPixels;
        info.screenHeight = dm.heightPixels;
        info.windowWidth = dm.widthPixels;
        info.windowHeight = dm.heightPixels;
        info.pixelRatio = dm.density;

        Log.d(TAG, "build: real metrics width=" + dm.widthPixels + " height=" + dm.heightPixels + " density=" + dm.density);

        int safeLeft = 0, safeTop = 0, safeRight = info.screenWidth, safeBottom = info.screenHeight;

        //------------------------------------------------------------------
        // 2. Get status bar and navigation bar heights (fallback)
        //------------------------------------------------------------------
        int statusBarId = res.getIdentifier("status_bar_height", "dimen", "android");
        int statusH = statusBarId > 0 ? res.getDimensionPixelSize(statusBarId) : 0;
        info.statusBarHeight = statusH;

        int navBarId = res.getIdentifier("navigation_bar_height", "dimen", "android");
        int navBarH = navBarId > 0 ? res.getDimensionPixelSize(navBarId) : 0;

        Log.d(TAG, "build: statusBarHeight=" + statusH + " navBarHeight=" + navBarH);

        //------------------------------------------------------------------
        // 3. API 23+: WindowInsets
        //------------------------------------------------------------------
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            WindowInsets insets = decor.getRootWindowInsets();
            if (insets != null) {

                int insetLeft = insets.getSystemWindowInsetLeft();
                int insetTop = insets.getSystemWindowInsetTop();
                int insetRight = insets.getSystemWindowInsetRight();
                int insetBottom = insets.getSystemWindowInsetBottom();

                boolean immersive = isImmersive(decor);
                if (immersive) {
                    insetBottom = 0; // fix ghost bottom inset artifact
                }

                Log.d(TAG, "build: WindowInsets left=" + insetLeft + " top=" + insetTop + " right=" + insetRight + " bottom=" + insetBottom + " immersive=" + immersive);

                //------------------------------------------------------------------
                // 4. API 28+: notch / screen cutout
                //------------------------------------------------------------------
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    DisplayCutout cutout = insets.getDisplayCutout();
                    if (cutout != null) {
                        Log.d(TAG, "build: DisplayCutout safeInsets l=" + cutout.getSafeInsetLeft() + " t=" + cutout.getSafeInsetTop() + " r=" + cutout.getSafeInsetRight() + " b=" + cutout.getSafeInsetBottom());
                        insetLeft = Math.max(insetLeft, cutout.getSafeInsetLeft());
                        insetTop = Math.max(insetTop, cutout.getSafeInsetTop());
                        insetRight = Math.max(insetRight, cutout.getSafeInsetRight());
                        insetBottom = Math.max(insetBottom, cutout.getSafeInsetBottom());
                    }
                }

                if ((insetLeft | insetTop | insetRight | insetBottom) != 0) {
                    safeLeft = insetLeft;
                    safeTop = insetTop;
                    safeRight = info.screenWidth - insetRight;
                    safeBottom = info.screenHeight - insetBottom;

                    Log.d(TAG, String.format("build: using insets -> safeArea=[%d,%d,%d,%d]", safeLeft, safeTop, safeRight, safeBottom));
                    return finalize(info, safeLeft, safeTop, safeRight, safeBottom);
                }
            } else {
                Log.d(TAG, "build: getRootWindowInsets returned null");
            }
        }

        //------------------------------------------------------------------
        // 5. API 21 fallback: visibleFrame
        //------------------------------------------------------------------
        Rect rect = new Rect();
        decor.getWindowVisibleDisplayFrame(rect);

        Log.d(TAG, "build: visibleFrame rect=" + rect.toShortString());

        if (rect.width() > 0 && rect.height() > 0 && rect.top >= statusH * 0.5f) {
            Log.d(TAG, "build: using visibleFrame as safe area");
            return finalize(info, rect.left, rect.top, rect.right, rect.bottom);
        }

        //------------------------------------------------------------------
        // 6. Final fallback: resource-based inference
        //------------------------------------------------------------------
        safeTop = statusH;
        safeBottom = info.screenHeight - navBarH;

        Log.d(TAG, String.format("build: final fallback safeTop=%d safeBottom=%d", safeTop, safeBottom));
        return finalize(info, safeLeft, safeTop, safeRight, safeBottom);
    }

    private static boolean isImmersive(View decor) {
        int flags = decor.getSystemUiVisibility();
        return (flags & (View.SYSTEM_UI_FLAG_FULLSCREEN | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY)) != 0;
    }

    private static WindowInfo finalize(WindowInfo info, int left, int top, int right, int bottom) {

        left = clamp(left, 0, info.screenWidth);
        right = clamp(right, 0, info.screenWidth);
        top = clamp(top, 0, info.screenHeight);
        bottom = clamp(bottom, 0, info.screenHeight);

        info.safeArea = new SafeArea(left, top, right, bottom);
        info.safeAreaBottom = info.screenHeight - bottom;

        Log.d(TAG, String.format("finalize: clamped safeArea=[%d,%d,%d,%d] safeAreaBottom=%d", left, top, right, bottom, info.safeAreaBottom));

        return info;
    }

    private static int clamp(int v, int min, int max) {
        if (v < min) return min;
        if (v > max) return max;
        return v;
    }
}
