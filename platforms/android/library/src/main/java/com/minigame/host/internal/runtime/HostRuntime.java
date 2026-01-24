package com.minigame.host.internal.runtime;

import android.app.Activity;
import android.view.Surface;

final class HostRuntime {
    final int hostId;
    Activity activity;
    Surface surface;

    HostRuntime(int hostId, Activity activity, Surface surface) {
        this.hostId = hostId;
        this.activity = activity;
        this.surface = surface;
    }
}
