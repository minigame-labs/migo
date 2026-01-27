package com.migo.host.internal.runtime;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Registry for managing active HostRuntime instances.
 * <p>
 * Thread-safe registry that maps host IDs to their runtime contexts.
 */
public final class HostRuntimeRegistry {

    private static final ConcurrentHashMap<Integer, HostRuntime> runtimes = new ConcurrentHashMap<>();

    private HostRuntimeRegistry() {
        // Prevent instantiation
    }

    /**
     * Register a new runtime.
     *
     * @param runtime The runtime to register
     */
    public static void register(HostRuntime runtime) {
        if (runtime != null) {
            runtimes.put(runtime.getHostId(), runtime);
        }
    }

    /**
     * Unregister a runtime by host ID.
     *
     * @param hostId The host ID to unregister
     * @return The removed runtime, or null if not found
     */
    public static HostRuntime unregister(int hostId) {
        return runtimes.remove(hostId);
    }

    /**
     * Get a runtime by host ID.
     *
     * @param hostId The host ID
     * @return The runtime, or null if not found
     */
    public static HostRuntime get(int hostId) {
        return runtimes.get(hostId);
    }

    /**
     * Get any available runtime.
     * <p>
     * Useful for operations that don't require a specific host context.
     *
     * @return Any runtime, or null if none registered
     */
    public static HostRuntime getAny() {
        for (HostRuntime runtime : runtimes.values()) {
            if (runtime.isValid()) {
                return runtime;
            }
        }
        return null;
    }

    /**
     * Get the number of registered runtimes.
     *
     * @return Runtime count
     */
    public static int size() {
        return runtimes.size();
    }

    /**
     * Clear all registered runtimes.
     */
    public static void clear() {
        runtimes.clear();
    }
}
