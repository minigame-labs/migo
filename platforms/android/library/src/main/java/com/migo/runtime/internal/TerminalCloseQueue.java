package com.migo.runtime.internal;

import java.util.Collections;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;

/** Captures terminal-close targets and deduplicates posts while one is pending. */
final class TerminalCloseQueue<T> {
    interface Poster {
        boolean post(Runnable runnable);
    }

    private final Set<T> pending =
            Collections.newSetFromMap(new ConcurrentHashMap<>());

    boolean schedule(T target, Poster poster, Consumer<T> close) {
        if (target == null) return false;
        if (!pending.add(target)) return true;
        boolean posted;
        try {
            posted = poster.post(() -> {
                try {
                    close.accept(target);
                } finally {
                    pending.remove(target);
                }
            });
        } catch (RuntimeException error) {
            pending.remove(target);
            throw error;
        }
        if (!posted) pending.remove(target);
        return posted;
    }
}
