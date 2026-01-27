package com.migo.host;

/**
 * Configuration options for initializing a MiniGame host.
 * <p>
 * Use the builder pattern to create instances:
 * <pre>{@code
 * InitOption options = new InitOption.Builder()
 *     .setAppTmpDir(cacheDir.getAbsolutePath())
 *     .setDpi(displayMetrics.density)
 *     .setTargetFps(60)
 *     .setDebugEnabled(BuildConfig.DEBUG)
 *     .build();
 * }</pre>
 */
public final class InitOption {

    /**
     * Log level for the native engine.
     */
    public enum LogLevel {
        /** Log everything including trace messages */
        TRACE,
        /** Log debug and above */
        DEBUG,
        /** Log warnings and above (default) */
        WARN,
        /** Log errors only */
        ERROR,
        /** Disable logging */
        OFF
    }

    // Required fields
    private final String appTmpDir;
    private final float dpi;

    // Optional fields with defaults
    private final String codeCacheDir;
    private final int targetFps;
    private final boolean debugEnabled;
    private final LogLevel logLevel;
    private final int maxMemoryMB;

    private InitOption(Builder builder) {
        this.appTmpDir = builder.appTmpDir;
        this.dpi = builder.dpi;
        this.codeCacheDir = builder.codeCacheDir;
        this.targetFps = builder.targetFps;
        this.debugEnabled = builder.debugEnabled;
        this.logLevel = builder.logLevel;
        this.maxMemoryMB = builder.maxMemoryMB;
    }

    // Getters - accessed via JNI field access
    public String getAppTmpDir() { return appTmpDir; }
    public float getDpi() { return dpi; }
    public String getCodeCacheDir() { return codeCacheDir; }
    public int getTargetFps() { return targetFps; }
    public boolean isDebugEnabled() { return debugEnabled; }
    public LogLevel getLogLevel() { return logLevel; }
    public int getMaxMemoryMB() { return maxMemoryMB; }

    /**
     * Builder for creating InitOption instances.
     */
    public static final class Builder {
        // Required
        private String appTmpDir;
        private float dpi = 1.0f;

        // Optional with defaults
        private String codeCacheDir;
        private int targetFps = 60;
        private boolean debugEnabled = false;
        private LogLevel logLevel = LogLevel.WARN;
        private int maxMemoryMB = 512;

        public Builder() {}

        /**
         * Set the application temporary directory (required).
         * Typically use {@code context.getCacheDir().getAbsolutePath()}.
         *
         * @param dir Absolute path to temp directory
         * @return this builder
         */
        public Builder setAppTmpDir(String dir) {
            this.appTmpDir = dir;
            return this;
        }

        /**
         * Set the display density/DPI (required).
         * Typically use {@code displayMetrics.density}.
         *
         * @param dpi Display density
         * @return this builder
         */
        public Builder setDpi(float dpi) {
            this.dpi = dpi;
            return this;
        }

        /**
         * Set the code cache directory for compiled code.
         * Defaults to appTmpDir if not set.
         *
         * @param dir Absolute path to code cache directory
         * @return this builder
         */
        public Builder setCodeCacheDir(String dir) {
            this.codeCacheDir = dir;
            return this;
        }

        /**
         * Set the target frame rate.
         * Default: 60 FPS
         *
         * @param fps Target frames per second (30-120)
         * @return this builder
         */
        public Builder setTargetFps(int fps) {
            this.targetFps = Math.max(30, Math.min(120, fps));
            return this;
        }

        /**
         * Enable or disable debug mode.
         * Default: false
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
         * Default: WARN
         *
         * @param level Log level
         * @return this builder
         */
        public Builder setLogLevel(LogLevel level) {
            this.logLevel = level != null ? level : LogLevel.WARN;
            return this;
        }

        /**
         * Set the maximum memory limit for the engine in megabytes.
         * Default: 512 MB
         *
         * @param mb Memory limit in MB
         * @return this builder
         */
        public Builder setMaxMemoryMB(int mb) {
            this.maxMemoryMB = Math.max(64, mb);
            return this;
        }

        /**
         * Build the InitOption instance.
         *
         * @return Configured InitOption
         * @throws IllegalStateException if required fields are not set
         */
        public InitOption build() {
            if (appTmpDir == null || appTmpDir.isEmpty()) {
                throw new IllegalStateException("appTmpDir is required");
            }
            if (codeCacheDir == null || codeCacheDir.isEmpty()) {
                codeCacheDir = appTmpDir;
            }
            return new InitOption(this);
        }
    }
}
