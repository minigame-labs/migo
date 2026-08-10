package com.migo.runtime.internal.util;

import android.util.Log;

import com.migo.runtime.RuntimeConfig;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Internal logging utility for the Migo Runtime SDK.
 * <p>
 * Respects the log level configured in {@link RuntimeConfig}. Until this class
 * was given {@link #registerSession}, nothing applied that setting at all: the
 * only mutator had no callers, so every SDK record was filtered at the hardcoded
 * {@code WARN} default whatever the embedder configured.
 * <p>
 * The level is the most verbose any live session asked for, and that is the whole
 * design. These records carry one tag and no session id — {@code Log.d(TAG, msg)}
 * from a manager, a callback thread, the main thread — so a record cannot be
 * attributed to a session and cannot be filtered per session. What can be
 * guaranteed is the direction that matters: a session starting with {@code OFF}
 * must not silence a live session that asked for {@code DEBUG}, because
 * diagnostics destroyed by an unrelated session are diagnostics nobody can get
 * back. The engine side can do better and does — its host thread carries a
 * binding, see {@code shared::log_level} — because a record there has a thread to
 * be attributed to.
 *
 * @hide
 */
public final class Logger {

    private static final String TAG = "MigoRuntime";

    /** Applies while no session is live, matching the engine's own default. */
    private static final int DEFAULT_LEVEL = RuntimeConfig.LogLevel.WARN.ordinal();

    /**
     * The level in force, cached so the check on every call is one field read.
     * <p>
     * Recomputed only when a session registers or unregisters. Deriving it from
     * the map per call would put a concurrent read on a path taken thousands of
     * times per second, to answer a question that changes twice per session.
     */
    private static volatile int sLogLevel = DEFAULT_LEVEL;

    /** Each live session's configured level, by session id. */
    private static final Map<Integer, Integer> sSessionLevels = new ConcurrentHashMap<>();

    private Logger() {}

    /**
     * Record that {@code sessionId} is live and configured for {@code level}.
     *
     * <p>Called at session creation. A repeat for the same id replaces its entry
     * rather than adding one, so a session cannot end up counted twice and hold
     * the level open after it closes.
     *
     * @param sessionId The session ID
     * @param level     That session's configured level; {@code null} means the
     *                  default, which is what an embedder who set nothing gets
     */
    public static void registerSession(int sessionId, RuntimeConfig.LogLevel level) {
        sSessionLevels.put(sessionId, level != null ? level.ordinal() : DEFAULT_LEVEL);
        republish();
    }

    /**
     * Forget {@code sessionId}, so its level stops holding the level open.
     *
     * @param sessionId The session ID
     */
    public static void unregisterSession(int sessionId) {
        sSessionLevels.remove(sessionId);
        republish();
    }

    /**
     * The most verbose level any live session asked for, or the default when none
     * is.
     *
     * <p>Ordinals ascend from {@code TRACE(0)} to {@code OFF(5)}, matching the
     * engine's, so the most verbose is the smallest and this is a minimum.
     */
    private static void republish() {
        int joined = Integer.MAX_VALUE;
        for (int level : sSessionLevels.values()) {
            if (level < joined) {
                joined = level;
            }
        }
        sLogLevel = joined == Integer.MAX_VALUE ? DEFAULT_LEVEL : joined;
    }

    /**
     * The level every check below reads.
     *
     * <p>One named read point rather than twelve field references: it is what makes
     * the cached value and the checks provably the same thing, and it is what a test
     * can observe without a Robolectric shadow of {@code android.util.Log}.
     * Package-private, so it is not surface an embedder can depend on.
     */
    static int effectiveLevel() {
        return sLogLevel;
    }

    /**
     * Log a trace message.
     *
     * @param message Log message
     */
    public static void t(String message) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.TRACE.ordinal()) {
            Log.v(TAG, message);
        }
    }

    /**
     * Log a trace message with format.
     *
     * @param format Format string
     * @param args   Format arguments
     */
    public static void t(String format, Object... args) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.TRACE.ordinal()) {
            Log.v(TAG, String.format(format, args));
        }
    }

    /**
     * Log a debug message.
     *
     * @param message Log message
     */
    public static void d(String message) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.DEBUG.ordinal()) {
            Log.d(TAG, message);
        }
    }

    /**
     * Log a debug message with format.
     *
     * @param format Format string
     * @param args   Format arguments
     */
    public static void d(String format, Object... args) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.DEBUG.ordinal()) {
            Log.d(TAG, String.format(format, args));
        }
    }

    /**
     * Log an info message.
     *
     * @param message Log message
     */
    public static void i(String message) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.INFO.ordinal()) {
            Log.i(TAG, message);
        }
    }

    /**
     * Log an info message with format.
     *
     * @param format Format string
     * @param args   Format arguments
     */
    public static void i(String format, Object... args) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.INFO.ordinal()) {
            Log.i(TAG, String.format(format, args));
        }
    }

    /**
     * Log a warning message.
     *
     * @param message Log message
     */
    public static void w(String message) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.WARN.ordinal()) {
            Log.w(TAG, message);
        }
    }

    /**
     * Log a warning message with format.
     *
     * @param format Format string
     * @param args   Format arguments
     */
    public static void w(String format, Object... args) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.WARN.ordinal()) {
            Log.w(TAG, String.format(format, args));
        }
    }

    /**
     * Log a warning message with exception.
     *
     * @param message   Log message
     * @param throwable Exception
     */
    public static void w(String message, Throwable throwable) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.WARN.ordinal()) {
            Log.w(TAG, message, throwable);
        }
    }

    /**
     * Log an error message.
     *
     * @param message Log message
     */
    public static void e(String message) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
            Log.e(TAG, message);
        }
    }

    /**
     * Log an error message with format.
     *
     * @param format Format string
     * @param args   Format arguments
     */
    public static void e(String format, Object... args) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
            Log.e(TAG, String.format(format, args));
        }
    }

    /**
     * Log an error message with exception.
     *
     * @param message   Log message
     * @param throwable Exception
     */
    public static void e(String message, Throwable throwable) {
        if (effectiveLevel() <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
            Log.e(TAG, message, throwable);
        }
    }
}
