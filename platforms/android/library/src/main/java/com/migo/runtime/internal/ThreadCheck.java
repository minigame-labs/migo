package com.migo.runtime.internal;

import android.os.Looper;

/**
 * Thread safety enforcement utility.
 * Call at the top of public SDK methods that must run on the main thread.
 */
public final class ThreadCheck {
    private ThreadCheck() {}

    /**
     * Throws if not called from the main (UI) thread.
     */
    public static void ensureMainThread() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            throw new IllegalStateException(
                "Must be called on the main thread. Current: " + Thread.currentThread().getName());
        }
    }
}
