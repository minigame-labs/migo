package com.migo.runtime.internal.platform;

import android.util.Log;

/**
 * Receives game log entries forwarded from JS.
 * <p>
 * By default logs are written to Android logcat. Host apps can set a custom
 * {@link GameLogHandler} to forward logs to their own analytics backend.
 *
 * @hide
 */
public class GameLogManager {

    private static final String TAG = "MigoGameLog";

    /**
     * Callback interface for host apps to receive game log entries.
     * <p>
     * Implement this interface and register it via
     * {@link GameLogManager#setHandler(GameLogHandler)} to intercept all
     * game log data for custom processing (e.g. upload to a backend,
     * forward to a third-party analytics SDK, etc.).
     */
    public interface GameLogHandler {
        /**
         * Called when the game reports a log entry.
         *
         * @param sessionId the session that produced the log
         * @param logJson   JSON string containing: level, key, value, commonInfo
         */
        void onLog(int sessionId, String logJson);
    }

    private final int sessionId;
    private volatile GameLogHandler handler;

    public GameLogManager(int sessionId) {
        this.sessionId = sessionId;
    }

    /**
     * Set a custom handler to receive log entries.
     * Pass {@code null} to revert to default logcat output.
     *
     * @param handler the handler, or null for default behaviour
     */
    public void setHandler(GameLogHandler handler) {
        this.handler = handler;
    }

    /**
     * Called from native (via NativeExports) when JS reports a log entry.
     *
     * @param logJson JSON string with level, key, value, commonInfo
     */
    public void reportLog(String logJson) {
        GameLogHandler h = handler;
        if (h != null) {
            h.onLog(sessionId, logJson);
        } else {
            Log.d(TAG, "[session=" + sessionId + "] " + logJson);
        }
    }

    public void destroy() {
        handler = null;
    }
}
