package com.minigame.host.internal.jni;

import android.content.Context;
import android.view.MotionEvent;
import android.view.Surface;

import com.minigame.host.InitOption;

import java.nio.ByteBuffer;

public final class HostNative {
    private HostNative() {
    }

    public static int init(Surface surface, InitOption option) {
        return HostJNI.init(surface, option);
    }

    public static void shutdown(int hostId) {
        HostJNI.shutdown(hostId);
    }

    public static int startGame(int hostId, String codeDir, String entry) {
        return HostJNI.modMain(hostId, codeDir, entry);
    }

    public static void updateSurface(int hostId, Surface surface) {
        HostJNI.updateSurface(hostId, surface);
    }

    public static void onShow(int hostId) {
        HostJNI.onShow(hostId);
    }

    public static void onHide(int hostId) {
        HostJNI.onHide(hostId);
    }

    public static void dispatchTouchEvent(int hostId, int actionMasked, long eventTime, int pointerCount, ByteBuffer buffer) {
        HostJNI.onTouchEvent(hostId, actionMasked, eventTime, pointerCount, buffer);
    }

    public static String version() {
        return HostJNI.version();
    }

    public static void onOpenSystemBluetoothSetting(int hostId, int code) {
        HostJNI.onOpenSystemBluetoothSetting(hostId, code);
    }

    public static void onUnzipDone(int hostId, int requestId) {
        HostJNI.onUnzipDone(hostId, requestId);
    }
}
