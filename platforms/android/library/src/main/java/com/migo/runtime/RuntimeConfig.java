package com.migo.runtime;

import android.content.Context;

/**
 * Configuration for initializing the Migo Runtime.
 * <p>
 * Use the {@link Builder} to create instances:
 *
 * <pre>{@code
 * RuntimeConfig config = new RuntimeConfig.Builder(context)
 *     .setTargetFps(60)
 *     .setDebugEnabled(BuildConfig.DEBUG)
 *     .setLogLevel(RuntimeConfig.LogLevel.DEBUG)
 *     .build();
 * }</pre>
 *
 * @since 1.0.0
 */
public final class RuntimeConfig {

    /**
     * Log level for the native engine.
     */
    public enum LogLevel {
        /** Log everything including trace messages */
        TRACE(0),
        /** Log debug and above */
        DEBUG(1),
        /** Log info and above */
        INFO(2),
        /** Log warnings and above (default) */
        WARN(3),
        /** Log errors only */
        ERROR(4),
        /** Disable logging */
        OFF(5);

        private final int value;

        LogLevel(int value) {
            this.value = value;
        }

        /**
         * Get the integer value for JNI.
         * @return Integer value
         */
        public int getValue() {
            return value;
        }
    }

    // Core settings
    private final String cacheDir;
    private final String filesDir;
    private final String codeCacheDir;
    private final float displayDensity;

    // Performance settings
    private final int targetFps;

    // Debug settings
    private final boolean debugEnabled;
    private final LogLevel logLevel;

    // Display settings
    private final boolean immersiveMode;

    // Safety settings
    private final boolean watchdogEnabled;
    private final int watchdogTimeoutSecs;
    private final boolean codeSigningEnabled;
    private final String codeSigningPubkey;

    private RuntimeConfig(Builder builder) {
        this.cacheDir = builder.cacheDir;
        this.filesDir = builder.filesDir;
        this.codeCacheDir = builder.codeCacheDir;
        this.displayDensity = builder.displayDensity;
        this.targetFps = builder.targetFps;
        this.debugEnabled = builder.debugEnabled;
        this.logLevel = builder.logLevel;
        this.immersiveMode = builder.immersiveMode;
        this.watchdogEnabled = builder.watchdogEnabled;
        this.watchdogTimeoutSecs = builder.watchdogTimeoutSecs;
        this.codeSigningEnabled = builder.codeSigningEnabled;
        this.codeSigningPubkey = builder.codeSigningPubkey;
    }

    // ==================== Getters ====================

    /** Get the cache directory path (may be cleared by system) */
    public String getCacheDir() { return cacheDir; }

    /** Get the files directory path (persistent storage) */
    public String getFilesDir() { return filesDir; }

    /** Get the code cache directory path */
    public String getCodeCacheDir() { return codeCacheDir; }

    /** Get the display density (pixels per dp) */
    public float getDisplayDensity() { return displayDensity; }

    /** Get the target frames per second */
    public int getTargetFps() { return targetFps; }

    /** Check if debug mode is enabled */
    public boolean isDebugEnabled() { return debugEnabled; }

    /** Get the log level */
    public LogLevel getLogLevel() { return logLevel; }

    /** Check whether full-screen immersive mode is enabled */
    public boolean isImmersiveMode() { return immersiveMode; }

    /** Check whether ANR watchdog is enabled */
    public boolean isWatchdogEnabled() { return watchdogEnabled; }

    /** Get ANR watchdog timeout in seconds */
    public int getWatchdogTimeoutSecs() { return watchdogTimeoutSecs; }

    /** Check whether code signing verification is enabled */
    public boolean isCodeSigningEnabled() { return codeSigningEnabled; }

    /** Get hex Ed25519 public key used by code signing verification (nullable) */
    public String getCodeSigningPubkey() { return codeSigningPubkey; }

    // ==================== JNI field access ====================

    /** @hide */
    int getLogLevelOrdinal() { return logLevel.ordinal(); }

    @Override
    public String toString() {
        return "RuntimeConfig{" +
                "cacheDir='" + cacheDir + '\'' +
                ", filesDir='" + filesDir + '\'' +
                ", targetFps=" + targetFps +
                ", debugEnabled=" + debugEnabled +
                ", logLevel=" + logLevel +
                ", watchdogEnabled=" + watchdogEnabled +
                ", watchdogTimeoutSecs=" + watchdogTimeoutSecs +
                ", codeSigningEnabled=" + codeSigningEnabled +
                '}';
    }

    // ==================== Builder ====================

    /**
     * Builder for creating RuntimeConfig instances.
     */
    public static final class Builder {
        // Core (auto-detected from context)
        private String cacheDir;
        private String filesDir;
        private String codeCacheDir;
        private float displayDensity = 1.0f;

        // Performance
        private int targetFps = 60;

        // Debug
        private boolean debugEnabled = false;
        private LogLevel logLevel = LogLevel.WARN;

        // Display
        private boolean immersiveMode = true;

        // Safety
        private boolean watchdogEnabled = true;
        private int watchdogTimeoutSecs = 10;
        private boolean codeSigningEnabled = true;
        private String codeSigningPubkey = null;

        /**
         * Create a new builder with required context.
         * <p>
         * Automatically extracts directories and display density from context.
         *
         * @param context Android context (Activity or Application)
         */
        public Builder(Context context) {
            if (context == null) {
                throw new IllegalArgumentException("Context cannot be null");
            }
            this.cacheDir = context.getCacheDir().getAbsolutePath();
            this.filesDir = context.getFilesDir().getAbsolutePath();
            this.codeCacheDir = this.cacheDir;
            this.displayDensity = context.getResources().getDisplayMetrics().density;
        }

        /**
         * Create a new builder with manual configuration.
         * <p>
         * Use this constructor when context is not available.
         *
         * @param cacheDir       Cache directory path
         * @param filesDir       Files directory path (persistent)
         * @param displayDensity Display density (dp ratio)
         */
        public Builder(String cacheDir, String filesDir, float displayDensity) {
            if (cacheDir == null || cacheDir.isEmpty()) {
                throw new IllegalArgumentException("cacheDir cannot be null or empty");
            }
            if (filesDir == null || filesDir.isEmpty()) {
                throw new IllegalArgumentException("filesDir cannot be null or empty");
            }
            this.cacheDir = cacheDir;
            this.filesDir = filesDir;
            this.codeCacheDir = cacheDir;
            this.displayDensity = displayDensity > 0 ? displayDensity : 1.0f;
        }

        /**
         * Set the code cache directory.
         * <p>
         * Used for storing compiled code. Defaults to the main cache directory.
         *
         * @param dir Absolute path to code cache directory
         * @return this builder
         */
        public Builder setCodeCacheDir(String dir) {
            if (dir != null && !dir.isEmpty()) {
                this.codeCacheDir = dir;
            }
            return this;
        }

        /**
         * Set the target frame rate.
         * <p>
         * Default: 60 FPS. Valid range: 30-120.
         *
         * @param fps Target frames per second
         * @return this builder
         */
        public Builder setTargetFps(int fps) {
            this.targetFps = Math.max(30, Math.min(120, fps));
            return this;
        }

        /**
         * Enable or disable debug mode.
         * <p>
         * Default: false. Enable for development, disable for production.
         *
         * @param enabled true to enable debug features
         * @return this builder
         */
        public Builder setDebugEnabled(boolean enabled) {
            this.debugEnabled = enabled;
            return this;
        }

        /**
         * Set the log level for native code.
         * <p>
         * Default: WARN. Use DEBUG or TRACE for development.
         *
         * @param level Log level
         * @return this builder
         */
        public Builder setLogLevel(LogLevel level) {
            this.logLevel = level != null ? level : LogLevel.WARN;
            return this;
        }

        /**
         * Enable or disable full-screen immersive mode.
         * <p>
         * Default: true. Set to false when embedding the game view
         * alongside other UI elements (e.g., in a feed or partial-screen layout).
         *
         * @param enabled true to enter immersive mode on session creation
         * @return this builder
         */
        public Builder setImmersiveMode(boolean enabled) {
            this.immersiveMode = enabled;
            return this;
        }

        /**
         * Enable or disable the native watchdog.
         * <p>
         * Default: enabled.
         */
        public Builder setWatchdogEnabled(boolean enabled) {
            this.watchdogEnabled = enabled;
            return this;
        }

        /**
         * Set watchdog timeout in seconds.
         * <p>
         * Values are clamped to [5, 120]. Default: 10.
         */
        public Builder setWatchdogTimeoutSecs(int timeoutSecs) {
            this.watchdogTimeoutSecs = Math.max(5, Math.min(120, timeoutSecs));
            return this;
        }

        /**
         * Enable or disable code signing verification.
         * <p>
         * Default: enabled.
         */
        public Builder setCodeSigningEnabled(boolean enabled) {
            this.codeSigningEnabled = enabled;
            return this;
        }

        /**
         * Set Ed25519 public key (hex, 64 chars) for code signing verification.
         * <p>
         * Required when code signing is enabled.
         */
        public Builder setCodeSigningPubkey(String pubkeyHex) {
            if (pubkeyHex == null) {
                this.codeSigningPubkey = null;
            } else {
                String trimmed = pubkeyHex.trim();
                this.codeSigningPubkey = trimmed.isEmpty() ? null : trimmed;
            }
            return this;
        }

        /**
         * Build the RuntimeConfig instance.
         *
         * @return Configured RuntimeConfig
         * @throws IllegalStateException if required fields are not set
         */
        public RuntimeConfig build() {
            if (cacheDir == null || cacheDir.isEmpty()) {
                throw new IllegalStateException("cacheDir is required");
            }
            return new RuntimeConfig(this);
        }
    }
}
