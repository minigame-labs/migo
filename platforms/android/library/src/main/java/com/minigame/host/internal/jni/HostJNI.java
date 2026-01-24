package com.minigame.host.internal.jni;

import android.content.Context;

import androidx.annotation.RestrictTo;

import com.minigame.host.InitOption;

import java.nio.ByteBuffer;

final class HostJNI {
    static {
        System.loadLibrary("minigame_host");
    }

    static native String version();

    static native int init(Object surface, InitOption option);

    static native void shutdown(int hostId);

    static native void onOpenSystemBluetoothSetting(int hostId, int enabled);

    static native void onShow(int hostId);

    static native void onHide(int hostId);

    static native void updateSurface(int hostId, Object surface);

    static native void onUnzipDone(int hostId, int requestId);

    static native void onTouchEvent(int hostId, int actionMasked, long eventTime, int pointerCount, ByteBuffer buffer);

    static native int modMain(int hostId, String codeDir, String entry);
}