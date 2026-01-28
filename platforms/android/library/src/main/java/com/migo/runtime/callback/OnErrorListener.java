package com.migo.runtime.callback;

import com.migo.runtime.ErrorCode;

/**
 * Listener for runtime errors.
 * <p>
 * Implement this interface to receive error notifications that don't throw exceptions.
 *
 * <pre>{@code
 * session.setOnErrorListener((errorCode, message, recoverable) -> {
 *     if (recoverable) {
 *         Log.w(TAG, "Recoverable error: " + message);
 *     } else {
 *         Log.e(TAG, "Fatal error: " + message);
 *         finish();
 *     }
 * });
 * }</pre>
 *
 * @since 1.0.0
 */
public interface OnErrorListener {

    /**
     * Called when an error occurs during runtime.
     * <p>
     * This is called on the main thread.
     *
     * @param errorCode   Error code from {@link ErrorCode}
     * @param message     Human-readable error message
     * @param recoverable true if the runtime can continue after this error
     */
    void onError(int errorCode, String message, boolean recoverable);
}
