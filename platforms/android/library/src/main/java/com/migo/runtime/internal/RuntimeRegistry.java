
import java.util.concurrent.ConcurrentHashMap;

/**
 * Registry for managing active runtime contexts.
 * <p>
 * Thread-safe registry that maps session IDs to their runtime contexts.
 *
 * @hide
 */
public final class RuntimeRegistry {

    private static final ConcurrentHashMap<Integer, RuntimeContext> sContexts = new ConcurrentHashMap<>();

    private RuntimeRegistry() {}

    /**
     * Register a new runtime context.
     *
     * @param context The context to register
     */
    public static void register(RuntimeContext context) {
        if (context != null) {
            sContexts.put(context.getSessionId(), context);
        }
    }

    /**
     * Unregister a context by session ID.
     *
     * @param sessionId The session ID to unregister
     * @return The removed context, or null if not found
     */
    public static RuntimeContext unregister(int sessionId) {
        return sContexts.remove(sessionId);
    }

    /**
     * Get a context by session ID.
     *
     * @param sessionId The session ID
     * @return The context, or null if not found
     */
    public static RuntimeContext get(int sessionId) {
        return sContexts.get(sessionId);
    }

    /**
     * Get any available valid context.
     * <p>
     * Useful for operations that don't require a specific session context.
     *
     * @return Any valid context, or null if none available
     */
    public static RuntimeContext getAny() {
        for (RuntimeContext context : sContexts.values()) {
            if (context.isValid()) {
                return context;
            }
        }
        return null;
    }

    /**
     * Get the number of registered contexts.
     *
     * @return Context count
     */
    public static int size() {
        return sContexts.size();
    }

    /**
     * Clear all registered contexts.
     */
    public static void clear() {
        sContexts.clear();
    }

    /**
     * Check if a session is registered.
     *
     * @param sessionId The session ID to check
     * @return true if registered
     */
    public static boolean contains(int sessionId) {
        return sContexts.containsKey(sessionId);
    }
}
