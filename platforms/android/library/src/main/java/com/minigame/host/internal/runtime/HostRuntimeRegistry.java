package com.minigame.host.internal.runtime;

import android.app.Activity;
import android.util.SparseArray;
import android.view.Surface;

public final class HostRuntimeRegistry {

    private static final SparseArray<HostRuntime> HOSTS = new SparseArray<>();

    public static void register(int hostId, Activity activity, Surface surface) {
        HOSTS.put(hostId, new HostRuntime(hostId, activity, surface));
    }

    public static void unregister(int hostId) {
        HOSTS.remove(hostId);
    }

    private static HostRuntime get(int hostId) {
        return HOSTS.get(hostId);
    }

    public static Activity getActivity(int hostId) {
        HostRuntime rt = get(hostId);
        if (rt == null) return null;
        return rt.activity;
    }
}
