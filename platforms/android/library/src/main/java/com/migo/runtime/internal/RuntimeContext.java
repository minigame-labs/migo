
import android.app.Activity;

import java.lang.ref.WeakReference;

/**
 * Runtime context for a game session.
 * <p>
 * Holds weak references to Android components to prevent memory leaks.
 *
 * @hide
 */
public final class RuntimeContext {

    private final int sessionId;
    private final WeakReference<Activity> activityRef;
    private final long createdTime;

    /**
     * Create a new runtime context.
     *
     * @param sessionId The session ID
     * @param activity  The activity associated with this session (may be null)
     */
    public RuntimeContext(int sessionId, Activity activity) {
        this.sessionId = sessionId;
        this.activityRef = new WeakReference<>(activity);
        this.createdTime = System.currentTimeMillis();
    }

    /**
     * Get the session ID.
     *
     * @return The session ID
     */
    public int getSessionId() {
        return sessionId;
    }

    /**
     * Get the associated Activity.
     *
     * @return The Activity, or null if it has been garbage collected or not set
     */
    public Activity getActivity() {
        return activityRef.get();
    }

    /**
     * Get the creation time.
     *
     * @return Creation timestamp in milliseconds
     */
    public long getCreatedTime() {
        return createdTime;
    }

    /**
     * Check if the context is still valid.
     * <p>
     * A context is valid if it either:
     * - Has no Activity reference (headless mode), or
     * - Has a valid Activity that is not finishing/destroyed
     *
     * @return true if valid
     */
    public boolean isValid() {
        Activity activity = activityRef.get();
        if (activity == null) {
            // Headless mode - always valid
            return true;
        }
        return !activity.isFinishing() && !activity.isDestroyed();
    }

    /**
     * Check if this context has an Activity.
     *
     * @return true if Activity is present and not GC'd
     */
    public boolean hasActivity() {
        return activityRef.get() != null;
    }
}
