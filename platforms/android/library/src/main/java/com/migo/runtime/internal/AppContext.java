
import android.content.Context;

/**
 * Global application context holder.
 * <p>
 * Provides access to the application context from anywhere in the SDK.
 * Thread-safe initialization using double-checked locking.
 *
 * @hide
 */
public final class AppContext {

    private static volatile Context sAppContext;
    private static final Object LOCK = new Object();

    private AppContext() {}

    /**
     * Initialize the app context.
     * <p>
     * This should be called early in the application lifecycle, typically
     * from the SDK initialization code.
     *
     * @param context Any context (Application or Activity)
     */
    public static void init(Context context) {
        if (sAppContext == null && context != null) {
            synchronized (LOCK) {
                if (sAppContext == null) {
                    sAppContext = context.getApplicationContext();
                }
            }
        }
    }

    /**
     * Get the application context.
     *
     * @return The application context
     * @throws IllegalStateException if not initialized
     */
    public static Context get() {
        Context ctx = sAppContext;
        if (ctx == null) {
            throw new IllegalStateException("MigoRuntime not initialized. " +
                    "Call MigoRuntime.createSession() first.");
        }
        return ctx;
    }

    /**
     * Get the application context, or null if not initialized.
     *
     * @return The application context, or null
     */
    public static Context getOrNull() {
        return sAppContext;
    }

    /**
     * Check if the context has been initialized.
     *
     * @return true if initialized
     */
    public static boolean isInitialized() {
        return sAppContext != null;
    }
}
