package com.migo.runtime;

/**
 * Error codes for the Migo Runtime SDK.
 * <p>
 * All error codes are negative integers. Success is indicated by code 0.
 *
 * @since 1.0.0
 */
public final class ErrorCode {

    private ErrorCode() {}

    // ==================== Success ====================
    /** Operation completed successfully */
    public static final int SUCCESS = 0;

    // ==================== Initialization Errors (-1xxx) ====================
    /** Generic initialization failure */
    public static final int ERR_INIT_FAILED = -1000;
    /** Surface is null or invalid */
    public static final int ERR_INVALID_SURFACE = -1001;
    /** Configuration is null or invalid */
    public static final int ERR_INVALID_CONFIG = -1002;
    /** Native library failed to load */
    public static final int ERR_NATIVE_LOAD_FAILED = -1003;

    // ==================== Runtime Errors (-2xxx) ====================
    /** Session has been destroyed */
    public static final int ERR_SESSION_DESTROYED = -2000;
    /** Game code directory not found */
    public static final int ERR_CODE_DIR_NOT_FOUND = -2002;
    /** Entry point file not found */
    public static final int ERR_ENTRY_NOT_FOUND = -2003;
    /** JavaScript execution error */
    public static final int ERR_JS_EXECUTION = -2004;

    // ==================== Platform Errors (-5xxx) ====================
    /** Activity is null or finishing */
    public static final int ERR_INVALID_ACTIVITY = -5004;

    /**
     * Get a human-readable message for an error code.
     *
     * @param code The error code
     * @return Human-readable error message
     */
    public static String getMessage(int code) {
        switch (code) {
            case SUCCESS: return "Success";
            case ERR_INIT_FAILED: return "Initialization failed";
            case ERR_INVALID_SURFACE: return "Invalid surface";
            case ERR_INVALID_CONFIG: return "Invalid configuration";
            case ERR_NATIVE_LOAD_FAILED: return "Native library load failed";
            case ERR_SESSION_DESTROYED: return "Session destroyed";
            case ERR_CODE_DIR_NOT_FOUND: return "Code directory not found";
            case ERR_ENTRY_NOT_FOUND: return "Entry point not found";
            case ERR_JS_EXECUTION: return "JavaScript execution error";
            case ERR_INVALID_ACTIVITY: return "Invalid activity";
            default: return "Unknown error (" + code + ")";
        }
    }

    /**
     * Check if a code indicates success.
     *
     * @param code The error code
     * @return true if code equals SUCCESS
     */
    public static boolean isSuccess(int code) {
        return code == SUCCESS;
    }
}
