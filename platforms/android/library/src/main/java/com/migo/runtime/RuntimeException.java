
/**
 * Exception thrown by the Migo Runtime SDK.
 * <p>
 * Contains an error code from {@link ErrorCode} and an optional detail message.
 *
 * <pre>{@code
 * try {
 *     session.startGame(codeDir, "game.js");
 * } catch (RuntimeException e) {
 *     if (e.getErrorCode() == ErrorCode.ERR_CODE_DIR_NOT_FOUND) {
 *         // Handle missing code directory
 *     }
 *     Log.e(TAG, "Error: " + e.getErrorCode() + " - " + e.getMessage());
 * }
 * }</pre>
 *
 * @since 1.0.0
 */
public class RuntimeException extends java.lang.RuntimeException {

    private final int errorCode;

    /**
     * Create a new exception with an error code.
     *
     * @param errorCode Error code from {@link ErrorCode}
     */
    public RuntimeException(int errorCode) {
        super(ErrorCode.getMessage(errorCode));
        this.errorCode = errorCode;
    }

    /**
     * Create a new exception with an error code and detail message.
     *
     * @param errorCode Error code from {@link ErrorCode}
     * @param detail    Additional detail about the error
     */
    public RuntimeException(int errorCode, String detail) {
        super(ErrorCode.getMessage(errorCode) + (detail != null ? ": " + detail : ""));
        this.errorCode = errorCode;
    }

    /**
     * Create a new exception with an error code and cause.
     *
     * @param errorCode Error code from {@link ErrorCode}
     * @param cause     The cause of this exception
     */
    public RuntimeException(int errorCode, Throwable cause) {
        super(ErrorCode.getMessage(errorCode), cause);
        this.errorCode = errorCode;
    }

    /**
     * Create a new exception with error code, detail, and cause.
     *
     * @param errorCode Error code from {@link ErrorCode}
     * @param detail    Additional detail about the error
     * @param cause     The cause of this exception
     */
    public RuntimeException(int errorCode, String detail, Throwable cause) {
        super(ErrorCode.getMessage(errorCode) + (detail != null ? ": " + detail : ""), cause);
        this.errorCode = errorCode;
    }

    /**
     * Get the error code.
     *
     * @return Error code from {@link ErrorCode}
     */
    public int getErrorCode() {
        return errorCode;
    }

    @Override
    public String toString() {
        return "MigoRuntimeException[code=" + errorCode + ", message=" + getMessage() + "]";
    }
}
