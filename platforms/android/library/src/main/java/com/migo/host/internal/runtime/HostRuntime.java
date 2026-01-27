package com.migo.host.internal.runtime;

import android.app.Activity;

import java.lang.ref.WeakReference;

/**
 * Runtime context for a host instance.
 * <p>
 * Holds weak references to Android components to prevent memory leaks.
 */
public final class HostRuntime {

    private final int hostId;
    private final WeakReference<Activity> activityRef;

    /**
     * Create a new runtime context.
     *
     * @param hostId   The host ID
     * @param activity The activity associated with this host
     */
    public HostRuntime(int hostId, Activity activity) {
        this.hostId = hostId;
        this.activityRef = new WeakReference<>(activity);
    }

    /**
     * Get the host ID.
     *
     * @return The host ID
     */
    public int getHostId() {
        return hostId;
    }

    /**
     * Get the associated Activity.
     *
     * @return The Activity, or null if it has been garbage collected
     */
    public Activity getActivity() {
        return activityRef.get();
    }

    /**
     * Check if the runtime is still valid (Activity not GC'd and not finishing).
     *
     * @return true if valid
     */
    public boolean isValid() {
        Activity activity = activityRef.get();
        return activity != null && !activity.isFinishing() && !activity.isDestroyed();
    }
}
