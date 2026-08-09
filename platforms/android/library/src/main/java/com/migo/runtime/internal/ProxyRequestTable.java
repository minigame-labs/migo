package com.migo.runtime.internal;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The requests a {@link ResultProxyActivity} still owes a result to, and the
 * tokens that name them.
 *
 * <p>A proxy launch hands its request here, puts the returned token in the
 * Intent, and the proxy Activity takes it back out when the result arrives.
 * The token is the only thing that survives the trip, so two properties are
 * the whole job:
 *
 * <ul>
 *   <li><b>A token is never reissued.</b> The counter this replaced was
 *       {@code 10000 + (int++ % 55000)}, which repeats after 55,000 launches
 *       and hands the second launch the first one's entry — the first request
 *       then never receives a result at all, because {@code put} dropped its
 *       callback. Fifty-five thousand image picks in one process is a long
 *       session, not an impossible one.
 *   <li><b>An entry leaves only when someone takes it.</b> This type has no
 *       clock, so it cannot expire anything: the eviction it replaced removed
 *       entries older than sixty seconds on every launch, which is exactly how
 *       long a user browsing a gallery takes, and the request they were
 *       browsing for then settled through nothing.
 * </ul>
 *
 * <p>Every entry is removed by exactly one of {@link #take}'s callers — the
 * result, the proxy's destruction, or the launch that failed to start — so
 * nothing accumulates without a clock to sweep it.
 */
final class ProxyRequestTable<T> {

    private final ConcurrentHashMap<Long, T> entries = new ConcurrentHashMap<>();

    /** The last token issued; the next is one more, and there is no wrap. */
    private final AtomicLong lastToken;

    ProxyRequestTable() {
        this(0L);
    }

    /**
     * Starts the token space just below {@code lastIssued + 1}.
     *
     * <p>Package-private and used only to place a test at the far end of the
     * space. A caller that can choose the starting point is a caller that can
     * reissue a token, which is the one thing this type exists to prevent, so
     * it is not public API.
     */
    ProxyRequestTable(long lastIssued) {
        this.lastToken = new AtomicLong(lastIssued);
    }

    /**
     * Records {@code request} under a fresh token.
     *
     * @throws IllegalStateException if the token space is exhausted, before
     *     anything is recorded — a request registered under a token that was
     *     not allocated is a request the proxy can never find.
     */
    long register(T request) {
        long token = allocate();
        entries.put(token, request);
        return token;
    }

    /** Removes and returns the request named by {@code token}, or null. */
    T take(long token) {
        return entries.remove(token);
    }

    /** How many requests are still owed a result. */
    int size() {
        return entries.size();
    }

    private long allocate() {
        while (true) {
            long last = lastToken.get();
            if (last == Long.MAX_VALUE) {
                // Permanent, not a wrap: an allocator that recovers on the next
                // call has reissued a token that may still be in the map.
                throw new IllegalStateException("proxy request tokens exhausted");
            }
            if (lastToken.compareAndSet(last, last + 1)) {
                return last + 1;
            }
        }
    }
}
