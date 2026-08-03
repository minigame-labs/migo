package com.migo.runtime.internal;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Predicate;

/** Shared best-effort cleanup primitives for terminal resource release. */
public final class ResourceCleanup {
    public static final class Retained<T> {
        private T resource;

        public Retained(T resource) {
            this.resource = resource;
        }

        public synchronized T get() {
            return resource;
        }

        public synchronized void replace(T replacement, DestroyAction<T> destroy) {
            if (resource != null) destroy.destroy(resource);
            resource = replacement;
        }

        public synchronized void destroy(DestroyAction<T> destroy) {
            if (resource == null) return;
            destroy.destroy(resource);
            resource = null;
        }
    }

    @FunctionalInterface
    public interface Action {
        void run();
    }

    @FunctionalInterface
    public interface DestroyAction<T> {
        void destroy(T resource);
    }

    private ResourceCleanup() {}

    /** Runs every independent action and throws one aggregate failure afterwards. */
    public static void runAll(Action... actions) {
        RuntimeException failure = null;
        for (Action action : actions) {
            try {
                action.run();
            } catch (RuntimeException error) {
                failure = append(failure, error);
            }
        }
        if (failure != null) throw failure;
    }

    /** Destroys every match, removing only entries whose destroy action succeeds. */
    public static <K, T> void destroyMatching(
            ConcurrentHashMap<K, T> resources,
            Predicate<K> matches,
            DestroyAction<T> destroy) {
        RuntimeException failure = null;
        for (Map.Entry<K, T> entry : resources.entrySet()) {
            if (!matches.test(entry.getKey())) continue;
            try {
                destroy.destroy(entry.getValue());
                resources.remove(entry.getKey(), entry.getValue());
            } catch (RuntimeException error) {
                failure = append(failure, error);
            }
        }
        if (failure != null) throw failure;
    }

    /** Returns null only when the release action confirms success by returning normally. */
    public static <T> T release(T resource, DestroyAction<T> destroy) {
        if (resource == null) return null;
        destroy.destroy(resource);
        return null;
    }

    private static RuntimeException append(RuntimeException aggregate, RuntimeException next) {
        if (aggregate == null) return next;
        if (aggregate != next) aggregate.addSuppressed(next);
        return aggregate;
    }
}
