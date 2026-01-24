package com.minigame.host;

import android.app.Activity;
import android.content.Context;
import android.view.Surface;

import com.minigame.host.internal.AppContext;
import com.minigame.host.internal.jni.HostNative;
import com.minigame.host.internal.runtime.HostRuntimeRegistry;
import com.minigame.host.internal.util.Utils;

import java.util.Objects;

public final class MiniGameSDK {
    private static final Object LOCK = new Object();
    private static volatile MiniGameSDK sInstance;

    public static MiniGameSDK getInstance() {
        if (sInstance == null) {
            synchronized (LOCK) {
                if (sInstance == null) {
                    sInstance = new MiniGameSDK();
                }
            }
        }
        return sInstance;
    }

    public HostHandle initialize(Surface surface, Activity activity, InitOption option) {
        Objects.requireNonNull(surface);
        Objects.requireNonNull(activity);

        if (option.isFullScreen()) {
            Utils.enterFullScreen(activity);
        }

        AppContext.init(activity);

        int hostId = HostNative.init(surface, option);
        if (hostId < 0) return null;

        HostRuntimeRegistry.register(hostId, activity, surface);
        return new HostHandle(hostId, option);
    }
}
