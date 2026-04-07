package com.migo.runtime;

/**
 * Exception thrown by the Migo runtime SDK.
 * Contains a structured error code, human-readable message, and recovery guidance.
 *
 * <pre>{@code
 * session.setListener(new GameSessionListener() {
 *     @Override
 *     public void onError(MigoException exception) {
 *         if (!exception.isRecoverable()) {
 *             showErrorDialog(exception.getMessage());
 *         }
 *         Log.e(TAG, exception.toString());
 *     }
 * });
 * }</pre>
 *
 */
public class MigoException extends java.lang.RuntimeException {
    private final int errorCode;
    private final boolean recoverable;

    public MigoException(int errorCode, String message) {
        this(errorCode, message, null, false);
    }

    public MigoException(int errorCode, String message, Throwable cause) {
        this(errorCode, message, cause, false);
    }

    public MigoException(int errorCode, String message, Throwable cause, boolean recoverable) {
        super(message, cause);
        this.errorCode = errorCode;
        this.recoverable = recoverable;
    }

    /** Migo error code. See {@link ErrorCode} for constants. */
    public int getErrorCode() { return errorCode; }

    /**
     * Whether this error is recoverable.
     * Recoverable errors (e.g., transient JS errors) allow the session to continue.
     * Non-recoverable errors (e.g., native panic, OOM) require session restart.
     */
    public boolean isRecoverable() { return recoverable; }

    @Override
    public String toString() {
        return "MigoException{code=" + errorCode
            + ", recoverable=" + recoverable
            + ", message=" + getMessage() + "}";
    }
}
