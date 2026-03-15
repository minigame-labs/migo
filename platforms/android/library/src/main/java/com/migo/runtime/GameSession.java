package com.migo.runtime;

import android.app.Activity;
import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.view.MotionEvent;
import android.view.Surface;
import android.view.View;

import com.migo.runtime.callback.GameSessionListener;
import com.migo.runtime.internal.NativeExports;
import com.migo.runtime.internal.NativeBridge;
import com.migo.runtime.internal.NativeMethods;
import com.migo.runtime.internal.RuntimeContext;
import com.migo.runtime.internal.RuntimeRegistry;
import com.migo.runtime.internal.TouchEventHandler;
import com.migo.runtime.internal.VsyncScheduler;
import com.migo.runtime.internal.platform.AudioFocusManager;

import java.io.Closeable;
import java.io.File;

/**
 * Represents an active game session.
 * <p>
 * A GameSession is created by {@link MigoRuntime#createSession} and manages the
 * lifecycle of a running game. Each session is isolated with its own file system
 * sandbox based on the game ID.
 *
 * <h3>Usage Example</h3>
 * <pre>{@code
 * // Create and start a game session
 * GameSession session = MigoRuntime.getInstance()
 *     .createSession(activity, surface, config, "my-game-id");
 *
 * session.setListener(listener);
 *
 * // Start the game from its code directory
 * session.startGame("game.js");  // Uses paths.getCodeDir()
 *
 * // Or start from custom directory
 * session.startGame("/custom/path", "game.js");
 *
 * // Handle touch events
 * surfaceView.setOnTouchListener((v, event) -> {
 *     session.dispatchTouchEvent(event);
 *     return true;
 * });
 *
 * // In onPause
 * session.pause();
 *
 * // In onResume
 * session.resume();
 *
 * // In onDestroy
 * session.close();
 * }</pre>
 *
 * <h3>File System Sandbox</h3>
 * <p>
 * Each game has isolated directories accessible via {@link #getPaths()}:
 * <ul>
 *   <li>{@code /user} → User data (saves, preferences)</li>
 *   <li>{@code /cache} → Cache files</li>
 *   <li>{@code /code} → Game code (read-only)</li>
 *   <li>{@code /tmp} → Temporary files (cleared on close)</li>
 * </ul>
 *
 * <h3>Thread Safety</h3>
 * <p>
 * This class is thread-safe. Methods can be called from any thread.
 *
 * @since 1.0.0
 */
public final class GameSession implements Closeable {

    private final int sessionId;
    private final String gameId;
    private final RuntimeConfig config;
    private final GamePaths paths;
    private final TouchEventHandler touchHandler;
    private final AudioFocusManager audioFocusManager;
    private final VsyncScheduler vsyncScheduler;
    private final Handler mainHandler;
    private final Object lock = new Object();

    private volatile boolean destroyed = false;
    private volatile boolean gameStarted = false;

    private final long creationNanos = System.nanoTime();
    private volatile long startupTimeMs = -1;

    private DebugOverlayView debugOverlay;
    private boolean debugOverlayAttached = false;
    private ConsoleLogView consoleLogView;

    // Callback
    private volatile GameSessionListener listener;

    /**
     * Create a new game session.
     * <p>
     * This constructor is package-private. Use {@link MigoRuntime#createSession} to create sessions.
     *
     * @param sessionId The native session ID
     * @param gameId    The unique game identifier
     * @param config    The runtime configuration
     * @param context   The context for system services
     */
    GameSession(int sessionId, String gameId, RuntimeConfig config, Context context) {
        this.sessionId = sessionId;
        this.gameId = gameId;
        this.config = config;
        this.paths = new GamePaths(config, gameId);
        this.touchHandler = new TouchEventHandler(config.getDisplayDensity());
        this.audioFocusManager = new AudioFocusManager(sessionId, context);
        this.vsyncScheduler = new VsyncScheduler(sessionId);
        this.mainHandler = new Handler(Looper.getMainLooper());

        // Ensure game directories exist
        this.paths.ensureDirectories();

        // Start listening for audio focus changes
        this.audioFocusManager.start();

        // Start Choreographer-driven VSync immediately
        this.vsyncScheduler.start();

        // Create debug overlay and console log viewer if debug mode is enabled
        if (config.isDebugEnabled()) {
            this.debugOverlay = new DebugOverlayView(context, sessionId);
            this.debugOverlay.startMonitoring();
            this.consoleLogView = new ConsoleLogView(context, sessionId);
        }

        // Register session for lifecycle callbacks from native
        NativeExports.registerSession(sessionId, this);

        // Register for native engine error callbacks (OOM, ANR, Panic, Timeout)
        NativeExports.registerErrorCallback(sessionId, new NativeExports.NativeErrorCallback() {
            @Override
            public void onNativeError(int errorCode, String message, String detail) {
                // All native fatal errors are non-recoverable
                String fullMessage = detail != null && !detail.isEmpty()
                        ? message + " — " + detail
                        : message;
                notifyError(errorCode, fullMessage, /* recoverable */ false);
            }

            @Override
            public void onExit() {
                notifyGameExit(0);
            }
        });
    }

    // ==================== Getters ====================

    /**
     * Get the session ID.
     *
     * @return The native session ID
     */
    public int getSessionId() {
        return sessionId;
    }

    /**
     * Get the game ID.
     *
     * @return The unique game identifier
     */
    public String getGameId() {
        return gameId;
    }

    /**
     * Get the game paths manager.
     * <p>
     * Provides access to isolated directories for this game.
     *
     * @return The GamePaths instance
     */
    public GamePaths getPaths() {
        return paths;
    }

    /**
     * Check if this session is still valid (not destroyed).
     *
     * @return true if the session is valid
     */
    public boolean isValid() {
        return !destroyed;
    }

    /**
     * Check if a game has been started.
     *
     * @return true if startGame() has been called successfully
     */
    public boolean isGameStarted() {
        return gameStarted;
    }

    /**
     * Get the debug overlay view, if debug mode is enabled.
     * <p>
     * The overlay is automatically attached as a WindowManager panel on the
     * first {@link #updateSurface} call, floating above the game SurfaceView.
     * No manual {@code addView()} is needed.
     *
     * @return The debug overlay view, or null if debug mode is disabled
     */
    public DebugOverlayView getDebugOverlay() {
        return debugOverlay;
    }

    // ==================== Game Control ====================

    /**
     * Start the game.
     * <p>
     * The native layer will:
     * <ul>
     *   <li>Create isolated directories for this game based on gameId</li>
     *   <li>Set up the virtual file system with proper permissions</li>
     *   <li>Load and execute the entry point module</li>
     * </ul>
     * <p>
     * Before calling this method:
     * <ul>
     *   <li>Deploy game code to {@code paths.getCodeDir()}</li>
     *   <li>Verify the entry point file exists</li>
     * </ul>
     *
     * @param entryPoint Entry point file (e.g., "game.js", "main.js")
     * @throws RuntimeException if the session is destroyed or game fails to start
     */
    public void startGame(String entryPoint) {
        ensureNotDestroyed();

        if (entryPoint == null || entryPoint.isEmpty()) {
            throw new RuntimeException(ErrorCode.ERR_ENTRY_NOT_FOUND, "entryPoint is null or empty");
        }

        // Optional: Validate code directory and entry point exist (for better error messages)
        File codeDir = paths.getCodeDir();
        if (!codeDir.exists() || !codeDir.isDirectory()) {
            throw new RuntimeException(ErrorCode.ERR_CODE_DIR_NOT_FOUND,
                    "Code directory not found: " + codeDir.getAbsolutePath());
        }

        File entry = new File(codeDir, entryPoint);
        if (!entry.exists() || !entry.isFile()) {
            throw new RuntimeException(ErrorCode.ERR_ENTRY_NOT_FOUND,
                    "Entry point not found: " + entry.getAbsolutePath());
        }

        // Native layer handles path generation from gameId
        int result = NativeMethods.modMain(sessionId, gameId, entryPoint);
        if (result != 0) {
            throw new RuntimeException(ErrorCode.ERR_JS_EXECUTION, "Native modMain returned " + result);
        }

        gameStarted = true;
    }

    /**
     * Start the game (non-throwing version).
     *
     * @param entryPoint Entry point file (e.g., "game.js")
     * @return {@link ErrorCode#SUCCESS} on success, or an error code
     */
    public int startGameSafe(String entryPoint) {
        try {
            startGame(entryPoint);
            return ErrorCode.SUCCESS;
        } catch (RuntimeException e) {
            return e.getErrorCode();
        } catch (Exception e) {
            return ErrorCode.ERR_JS_EXECUTION;
        }
    }

    // ==================== Lifecycle ====================

    /**
     * Pause the game (call when activity goes to background).
     */
    public void pause() {
        synchronized (lock) {
            if (destroyed) return;
            vsyncScheduler.stop();
            if (debugOverlay != null) {
                debugOverlay.stopMonitoring();
                debugOverlay.detachFromWindow();
                debugOverlayAttached = false;
            }
            if (consoleLogView != null) {
                consoleLogView.stopPolling();
                consoleLogView.detach();
            }
            NativeMethods.onHide(sessionId);
        }
        GameSessionListener l = listener;
        if (l != null) {
            l.onPaused();
        }
    }

    /**
     * Resume the game (call when activity comes to foreground).
     */
    public void resume() {
        synchronized (lock) {
            if (destroyed) return;
            vsyncScheduler.start();
            if (debugOverlay != null) {
                debugOverlay.startMonitoring();
                if (!debugOverlayAttached) {
                    tryAttachDebugOverlay();
                }
            }
            NativeMethods.onShow(sessionId);
            // Re-request audio focus in case it was permanently lost (e.g.
            // after a phone call). This ensures onAudioInterruptionEnd fires.
            audioFocusManager.requestFocusIfNeeded();
        }
        GameSessionListener l = listener;
        if (l != null) {
            l.onResumed();
        }
    }

    /**
     * Restart the game.
     */
    public void restart() {
        synchronized (lock) {
            if (destroyed) return;
            NativeMethods.onRestart(sessionId);
        }
    }

    /**
     * Update the rendering surface.
     * <p>
     * Call this when the surface is recreated (e.g., after configuration change).
     *
     * @param surface The new Surface object
     */
    public void updateSurface(Surface surface) {
        if (surface == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_SURFACE);
        }
        synchronized (lock) {
            if (destroyed) return;
            NativeMethods.updateSurface(sessionId, surface);

            // Auto-attach debug overlay as a WindowManager panel on first surface update.
            // At this point the Activity window is guaranteed to have a valid token.
            if (debugOverlay != null && !debugOverlayAttached) {
                tryAttachDebugOverlay();
            }
        }
    }

    /**
     * Destroy this session and release all resources.
     * <p>
     * After calling this method, the session cannot be used anymore.
     * This method is idempotent - calling it multiple times has no effect.
     * <p>
     * Temporary files are automatically cleaned up.
     */
    @Override
    public void close() {
        synchronized (lock) {
            if (destroyed) {
                return;
            }
            destroyed = true;
        }

        vsyncScheduler.stop();
        if (debugOverlay != null) {
            debugOverlay.stopMonitoring();
            debugOverlay.detachFromWindow();
        }
        if (consoleLogView != null) {
            consoleLogView.detach();
        }
        audioFocusManager.stop();

        // Destroy all per-session managers (unified cleanup)
        NativeExports.destroyAllManagers(sessionId);
        NativeExports.unregisterSession(sessionId);

        NativeMethods.shutdown(sessionId);
        RuntimeRegistry.unregister(sessionId);

        // Clean up temporary files
        paths.cleanupTemp();

        GameSessionListener l = listener;
        if (l != null) {
            l.onDestroyed();
        }
    }

    /**
     * Alias for {@link #close()}.
     */
    public void destroy() {
        close();
    }

    // ==================== Memory Warning ====================

    /**
     * Forward a memory warning to the game session.
     * <p>
     * Call this from your Activity's {@code onTrimMemory} or Application's
     * {@code onTrimMemory} callback to notify the game of memory pressure.
     *
     * <pre>{@code
     * @Override
     * public void onTrimMemory(int level) {
     *     super.onTrimMemory(level);
     *     if (session != null && session.isValid()) {
     *         session.dispatchMemoryWarning(level);
     *     }
     * }
     * }</pre>
     *
     * @param level Android ComponentCallbacks2 trim memory level
     *              (e.g., TRIM_MEMORY_RUNNING_MODERATE=5, TRIM_MEMORY_RUNNING_LOW=10,
     *              TRIM_MEMORY_RUNNING_CRITICAL=15)
     */
    public void dispatchMemoryWarning(int level) {
        synchronized (lock) {
            if (destroyed) return;
            NativeMethods.onMemoryWarning(sessionId, level);
        }
    }

    // ==================== Input ====================

    /**
     * Dispatch a touch event to the game.
     *
     * @param event The MotionEvent from the view
     * @return true if the event was handled
     */
    public boolean dispatchTouchEvent(MotionEvent event) {
        if (event == null) return false;
        synchronized (lock) {
            if (destroyed) return false;
            touchHandler.dispatch(sessionId, event);
            return true;
        }
    }

    // ==================== Callback ====================

    /**
     * Set the listener for session events.
     *
     * @param listener The listener, or null to remove
     */
    public void setListener(GameSessionListener listener) {
        this.listener = listener;
    }

    // ==================== Internal callbacks from native ====================

    /**
     * @hide Called from native code via NativeExports.onGameReady (potentially from a non-UI thread).
     * Records startup timing, updates the debug overlay, then posts the listener
     * callback to the main thread with double-check to handle session destruction
     * between post and dispatch.
     */
    public void notifyGameReady() {
        startupTimeMs = (System.nanoTime() - creationNanos) / 1_000_000;
        if (debugOverlay != null) {
            debugOverlay.setStartupTimeMs(startupTimeMs);
        }
        GameSessionListener l = listener;
        if (l == null) return;
        mainHandler.post(() -> {
            GameSessionListener l2 = listener;
            if (l2 != null && !destroyed) {
                l2.onGameReady();
            }
        });
    }

    /** @hide Called from native code (potentially from a non-UI thread). */
    void notifyGameExit(int exitCode) {
        GameSessionListener l = listener;
        if (l == null) return;
        mainHandler.post(() -> {
            GameSessionListener l2 = listener;
            if (l2 != null) {
                l2.onGameExit(exitCode);
            }
        });
    }

    /** @hide Called from native code (potentially from a non-UI thread). */
    void notifyError(int errorCode, String message, boolean recoverable) {
        GameSessionListener l = listener;
        if (l == null) return;
        mainHandler.post(() -> {
            GameSessionListener l2 = listener;
            if (l2 != null) {
                l2.onError(errorCode, message, recoverable);
            }
        });
    }

    // ==================== Debug ====================

    /**
     * Enable or disable the debug overlay at runtime.
     * <p>
     * Must be called on the main thread.
     *
     * @param enabled true to show debug overlay, false to hide it
     * @hide
     */
    public void setDebugEnabled(boolean enabled) {
        synchronized (lock) {
            if (destroyed) return;
            if (enabled) {
                if (debugOverlay == null) {
                    RuntimeContext ctx = RuntimeRegistry.get(sessionId);
                    if (ctx == null) return;
                    Context context = ctx.getActivity();
                    if (context == null) return;
                    debugOverlay = new DebugOverlayView(context, sessionId);
                    debugOverlay.startMonitoring();
                    consoleLogView = new ConsoleLogView(context, sessionId);
                    tryAttachDebugOverlay();
                }
            } else {
                if (debugOverlay != null) {
                    debugOverlay.stopMonitoring();
                    debugOverlay.detachFromWindow();
                    debugOverlay = null;
                    debugOverlayAttached = false;
                }
                if (consoleLogView != null) {
                    consoleLogView.stopPolling();
                    consoleLogView.detach();
                    consoleLogView = null;
                }
            }
        }
    }

    // ==================== Helpers ====================

    private void tryAttachDebugOverlay() {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) return;
        Activity activity = ctx.getActivity();
        if (activity == null || activity.isFinishing()) return;
        View decor = activity.getWindow().getDecorView();
        if (decor.getWindowToken() == null) return;
        debugOverlay.attachToWindow(decor);
        if (consoleLogView != null) {
            consoleLogView.attachButton(decor);
        }
        debugOverlayAttached = true;
    }

    private void ensureNotDestroyed() {
        if (destroyed) {
            throw new RuntimeException(ErrorCode.ERR_SESSION_DESTROYED);
        }
    }

    @Override
    public String toString() {
        return "GameSession{" +
                "sessionId=" + sessionId +
                ", gameId='" + gameId + '\'' +
                ", destroyed=" + destroyed +
                ", gameStarted=" + gameStarted +
                '}';
    }
}
