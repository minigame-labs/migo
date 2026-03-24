package com.migo.runtime;

import android.app.Activity;
import android.content.Context;
import android.os.Build;
import android.view.Surface;

import com.migo.runtime.internal.AppContext;
import com.migo.runtime.internal.NativeBridge;
import com.migo.runtime.internal.NativeMethods;
import com.migo.runtime.internal.RuntimeContext;
import com.migo.runtime.internal.RuntimeRegistry;
import com.migo.runtime.internal.platform.DisplayCompat;

/**
 * Main entry point for the Migo Runtime SDK.
 * <p>
 * The Migo Runtime is a lightweight JavaScript game runtime for Android. It provides
 * a high-performance environment for running JavaScript-based games with Canvas 2D
 * and WebGL rendering support.
 *
 * <h3>Quick Start</h3>
 * <pre>{@code
 * // In your Activity or Fragment
 * public class GameActivity extends Activity {
 *     private GameSession session;
 *
 *     @Override
 *     protected void onCreate(Bundle savedInstanceState) {
 *         super.onCreate(savedInstanceState);
 *
 *         // Initialize the runtime
 *         MigoRuntime runtime = MigoRuntime.getInstance();
 *
 *         // Create configuration
 *         RuntimeConfig config = new RuntimeConfig.Builder(this)
 *             .setDebugEnabled(BuildConfig.DEBUG)
 *             .build();
 *
 *         // Create a surface view for rendering
 *         SurfaceView surfaceView = new SurfaceView(this);
 *         setContentView(surfaceView);
 *
 *         surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
 *             @Override
 *             public void surfaceCreated(SurfaceHolder holder) {
 *                 // Create session when surface is ready
 *                 session = runtime.createSession(
 *                     GameActivity.this,
 *                     holder.getSurface(),
 *                     config,
 *                     "my-game-id"
 *                 );
 *
 *                 // Start the game
 *                 // Place game code into: session.getPaths().getCodeDir()
 *                 session.startGame("game.js");
 *             }
 *
 *             @Override
 *             public void surfaceChanged(SurfaceHolder holder, int f, int w, int h) {
 *                 if (session != null) {
 *                     session.updateSurface(holder.getSurface());
 *                 }
 *             }
 *
 *             @Override
 *             public void surfaceDestroyed(SurfaceHolder holder) {
 *                 if (session != null) {
 *                     session.close();
 *                     session = null;
 *                 }
 *             }
 *         });
 *
 *         // Handle touch events
 *         surfaceView.setOnTouchListener((v, event) -> {
 *             if (session != null) {
 *                 return session.dispatchTouchEvent(event);
 *             }
 *             return false;
 *         });
 *     }
 *
 *     @Override
 *     protected void onPause() {
 *         super.onPause();
 *         if (session != null) session.pause();
 *     }
 *
 *     @Override
 *     protected void onResume() {
 *         super.onResume();
 *         if (session != null) session.resume();
 *     }
 *
 *     @Override
 *     protected void onDestroy() {
 *         if (session != null) {
 *             session.close();
 *             session = null;
 *         }
 *         super.onDestroy();
 *     }
 * }
 * }</pre>
 *
 * <h3>Thread Safety</h3>
 * <p>
 * This class is thread-safe. All public methods can be called from any thread.
 *
 * @since 1.0.0
 */
public final class MigoRuntime {

    private static final Object LOCK = new Object();
    private static volatile MigoRuntime sInstance;

    private volatile boolean nativeLoaded = false;
    private String nativeVersion = "unknown";

    /**
     * Get the singleton instance of MigoRuntime.
     *
     * @return The MigoRuntime instance
     */
    public static MigoRuntime getInstance() {
        if (sInstance == null) {
            synchronized (LOCK) {
                if (sInstance == null) {
                    sInstance = new MigoRuntime();
                }
            }
        }
        return sInstance;
    }

    private MigoRuntime() {
        try {
            System.loadLibrary("migo");
            nativeLoaded = true;
            nativeVersion = NativeBridge.version();
            if (nativeVersion == null) {
                nativeVersion = "unknown";
            }
        } catch (UnsatisfiedLinkError e) {
            nativeLoaded = false;
            nativeVersion = "load_failed";
        }
    }

    // ==================== Version Info ====================

    /**
     * Get the SDK version string.
     *
     * @return SDK version (e.g., "1.0.0")
     */
    public String getVersion() {
        return BuildInfo.VERSION;
    }

    /**
     * Get the native engine version string.
     *
     * @return Native version (e.g., "0.1.0"), or "unknown" if not loaded
     */
    public String getNativeVersion() {
        return nativeVersion;
    }

    /**
     * Check if the native library is loaded.
     *
     * @return true if native library loaded successfully
     */
    public boolean isNativeLoaded() {
        return nativeLoaded;
    }

    /**
     * Get the minimum supported Android API level.
     *
     * @return Minimum API level (21 = Android 5.0)
     */
    public int getMinSdkVersion() {
        return 21;
    }

    // ==================== Session Creation ====================

    /**
     * Create a new game session with isolated file system.
     * <p>
     * This is the recommended way to create a session. It automatically sets up
     * full-screen immersive mode, initializes the application context, and creates
     * isolated directories for the game.
     *
     * @param activity The host Activity
     * @param surface  The rendering Surface
     * @param config   Runtime configuration
     * @param gameId   Unique game identifier (alphanumeric, underscore, hyphen, 1-64 chars)
     * @return A new GameSession
     * @throws RuntimeException if creation fails or gameId is invalid
     */
    public GameSession createSession(Activity activity, Surface surface, RuntimeConfig config, String gameId) {
        validateCreateSession(activity, surface, config);
        validateGameId(gameId);

        // Initialize app context
        AppContext.init(activity);

        // Enter full-screen immersive mode (if enabled)
        if (config.isImmersiveMode()) {
            enterImmersiveMode(activity);
        }

        // Create native session
        int sessionId = NativeMethods.init(surface, config);
        if (sessionId < 0) {
            throw new RuntimeException(ErrorCode.ERR_INIT_FAILED,
                    "Native init returned " + sessionId);
        }

        // Create Java session wrapper with gameId
        GameSession session = new GameSession(sessionId, gameId, config, activity);

        // Register runtime context
        RuntimeRegistry.register(new RuntimeContext(sessionId, activity));

        return session;
    }

    /**
     * Create a new game session without Activity binding.
     * <p>
     * Use this method when creating a session without an Activity, such as in a Service
     * or when using a standalone SurfaceView. Note that some features requiring Activity
     * (like permissions, system settings) will not work.
     *
     * @param context Application or Activity context
     * @param surface The rendering Surface
     * @param config  Runtime configuration
     * @param gameId  Unique game identifier (alphanumeric, underscore, hyphen, 1-64 chars)
     * @return A new GameSession
     * @throws RuntimeException if creation fails or gameId is invalid
     */
    public GameSession createSession(Context context, Surface surface, RuntimeConfig config, String gameId) {
        if (context == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_CONFIG, "context is null");
        }
        if (surface == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_SURFACE);
        }
        if (config == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_CONFIG);
        }
        validateGameId(gameId);

        if (!nativeLoaded) {
            throw new RuntimeException(ErrorCode.ERR_NATIVE_LOAD_FAILED);
        }

        // Initialize app context
        AppContext.init(context);

        // Create native session
        int sessionId = NativeMethods.init(surface, config);
        if (sessionId < 0) {
            throw new RuntimeException(ErrorCode.ERR_INIT_FAILED,
                    "Native init returned " + sessionId);
        }

        // Create Java session wrapper with gameId
        GameSession session = new GameSession(sessionId, gameId, config, context);

        // Register without Activity
        RuntimeRegistry.register(new RuntimeContext(sessionId, null));

        return session;
    }

    /**
     * Create a new game session with isolated file system.
     * <p>
     * Non-throwing version that returns a result wrapper.
     *
     * @param activity The host Activity
     * @param surface  The rendering Surface
     * @param config   Runtime configuration
     * @param gameId   Unique game identifier
     * @return Result containing either the session or an error code
     */
    public Result<GameSession> createSessionSafe(Activity activity, Surface surface, RuntimeConfig config, String gameId) {
        try {
            return Result.success(createSession(activity, surface, config, gameId));
        } catch (RuntimeException e) {
            return Result.failure(e.getErrorCode(), e.getMessage());
        } catch (Exception e) {
            return Result.failure(ErrorCode.ERR_INIT_FAILED, e.getMessage());
        }
    }

    // ==================== Utility Methods ====================

    /**
     * Check if the current device meets minimum requirements.
     *
     * @return true if device is supported
     */
    public boolean isDeviceSupported() {
        return Build.VERSION.SDK_INT >= 21 && nativeLoaded;
    }

    /**
     * Get the number of active sessions.
     *
     * @return Number of active sessions
     */
    public int getActiveSessionCount() {
        return RuntimeRegistry.size();
    }

    // ==================== Private Methods ====================

    private void validateCreateSession(Activity activity, Surface surface, RuntimeConfig config) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_ACTIVITY);
        }
        if (surface == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_SURFACE);
        }
        if (config == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_CONFIG);
        }
        if (!nativeLoaded) {
            throw new RuntimeException(ErrorCode.ERR_NATIVE_LOAD_FAILED);
        }
    }

    private void validateGameId(String gameId) {
        if (!GamePaths.isValidGameId(gameId)) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_CONFIG,
                    "Invalid gameId: must be 1-64 alphanumeric characters, underscore or hyphen");
        }
    }

    private void enterImmersiveMode(Activity activity) {
        // Use DisplayCompat for API 21+ compatibility
        DisplayCompat.enterImmersiveMode(activity);
    }

    // ==================== Inner Classes ====================

    /**
     * Build information constants.
     */
    public static final class BuildInfo {
        /** SDK version string */
        public static final String VERSION = "1.0.0";
        /** Build timestamp */
        public static final String BUILD_TIME = "2024-01-01T00:00:00Z";
        /** Git commit hash */
        public static final String COMMIT = "unknown";

        private BuildInfo() {}
    }

    /**
     * Result wrapper for non-throwing operations.
     *
     * @param <T> The success value type
     */
    public static final class Result<T> {
        private final T value;
        private final int errorCode;
        private final String errorMessage;

        private Result(T value, int errorCode, String errorMessage) {
            this.value = value;
            this.errorCode = errorCode;
            this.errorMessage = errorMessage;
        }

        static <T> Result<T> success(T value) {
            return new Result<>(value, ErrorCode.SUCCESS, null);
        }

        static <T> Result<T> failure(int errorCode, String message) {
            return new Result<>(null, errorCode, message);
        }

        /** Check if operation succeeded */
        public boolean isSuccess() {
            return errorCode == ErrorCode.SUCCESS;
        }

        /** Check if operation failed */
        public boolean isFailure() {
            return errorCode != ErrorCode.SUCCESS;
        }

        /** Get the success value (null if failed) */
        public T getValue() {
            return value;
        }

        /** Get the error code */
        public int getErrorCode() {
            return errorCode;
        }

        /** Get the error message (null if succeeded) */
        public String getErrorMessage() {
            return errorMessage;
        }

        /** Get value or throw if failed */
        public T getOrThrow() {
            if (isFailure()) {
                throw new RuntimeException(errorCode, errorMessage);
            }
            return value;
        }
    }
}
