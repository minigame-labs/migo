package com.migo.host;

import android.app.Activity;
import android.content.Context;
import android.view.Surface;

import com.migo.host.internal.AppContext;
import com.migo.host.internal.jni.HostNative;
import com.migo.host.internal.runtime.HostRuntime;
import com.migo.host.internal.runtime.HostRuntimeRegistry;
import com.migo.host.internal.util.Utils;

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

        Utils.enterFullScreen(activity);

        AppContext.init(activity);

        int hostId = HostNative.init(surface, option);
        if (hostId < 0) return null;

        HostRuntimeRegistry.register(new HostRuntime(hostId, activity));
        return new HostHandle(hostId, option);
    }
}
