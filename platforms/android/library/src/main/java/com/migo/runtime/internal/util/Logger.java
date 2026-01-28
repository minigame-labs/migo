
import android.util.Log;

import com.migo.runtime.RuntimeConfig;

/**
 * Internal logging utility for the Migo Runtime SDK.
 * <p>
 * Respects the log level configured in {@link RuntimeConfig}.
 *
 * @hide
 */
public final class Logger {

    private static final String TAG = "MigoRuntime";
    private static volatile int sLogLevel = RuntimeConfig.LogLevel.WARN.ordinal();

    private Logger() {}

    /**
     * Set the log level.
     *
     * @param level Log level ordinal
     */
    public static void setLogLevel(int level) {
        sLogLevel = level;
    }

    /**
     * Set the log level.
     *
     * @param level Log level
     */
    public static void setLogLevel(RuntimeConfig.LogLevel level) {
        sLogLevel = level != null ? level.ordinal() : RuntimeConfig.LogLevel.WARN.ordinal();
    }

    /**
     * Log a trace message.
     *
     * @param message Log message
     */
    public static void t(String message) {
        if (sLogLevel <= RuntimeConfig.LogLevel.TRACE.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.TRACE.ordinal()) {
            Log.v(TAG, String.format(format, args));
        }
    }

    /**
     * Log a debug message.
     *
     * @param message Log message
     */
    public static void d(String message) {
        if (sLogLevel <= RuntimeConfig.LogLevel.DEBUG.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.DEBUG.ordinal()) {
            Log.d(TAG, String.format(format, args));
        }
    }

    /**
     * Log an info message.
     *
     * @param message Log message
     */
    public static void i(String message) {
        if (sLogLevel <= RuntimeConfig.LogLevel.INFO.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.INFO.ordinal()) {
            Log.i(TAG, String.format(format, args));
        }
    }

    /**
     * Log a warning message.
     *
     * @param message Log message
     */
    public static void w(String message) {
        if (sLogLevel <= RuntimeConfig.LogLevel.WARN.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.WARN.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.WARN.ordinal()) {
            Log.w(TAG, message, throwable);
        }
    }

    /**
     * Log an error message.
     *
     * @param message Log message
     */
    public static void e(String message) {
        if (sLogLevel <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
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
        if (sLogLevel <= RuntimeConfig.LogLevel.ERROR.ordinal()) {
            Log.e(TAG, message, throwable);
        }
    }
}
