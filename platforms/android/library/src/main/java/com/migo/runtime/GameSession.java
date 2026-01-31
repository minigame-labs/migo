package com.migo.runtime;

import android.content.Context;
import android.view.MotionEvent;
import android.view.Surface;

import com.migo.runtime.callback.OnErrorListener;
import com.migo.runtime.callback.OnGameEventListener;
import com.migo.runtime.callback.OnLifecycleListener;
import com.migo.runtime.internal.NativeBridge;
import com.migo.runtime.internal.NativeMethods;
import com.migo.runtime.internal.RuntimeRegistry;
import com.migo.runtime.internal.TouchEventHandler;
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
 * session.setOnGameEventListener(eventListener);
 * session.setOnErrorListener(errorListener);
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
    private final Object lock = new Object();

    private volatile boolean destroyed = false;
    private volatile boolean gameStarted = false;

    // Callbacks
    private OnGameEventListener gameEventListener;
    private OnErrorListener errorListener;
    private OnLifecycleListener lifecycleListener;

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

        // Ensure game directories exist
        this.paths.ensureDirectories();

        // Start listening for audio focus changes
        this.audioFocusManager.start();
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
        if (!destroyed) {
            NativeMethods.onHide(sessionId);
            if (lifecycleListener != null) {
                lifecycleListener.onPaused();
            }
        }
    }

    /**
     * Resume the game (call when activity comes to foreground).
     */
    public void resume() {
        if (!destroyed) {
            NativeMethods.onShow(sessionId);
            // Re-request audio focus in case it was permanently lost (e.g.
            // after a phone call). This ensures onAudioInterruptionEnd fires.
            audioFocusManager.requestFocusIfNeeded();
            if (lifecycleListener != null) {
                lifecycleListener.onResumed();
            }
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
        ensureNotDestroyed();
        if (surface == null) {
            throw new RuntimeException(ErrorCode.ERR_INVALID_SURFACE);
        }
        NativeMethods.updateSurface(sessionId, surface);
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

        audioFocusManager.stop();
        NativeMethods.shutdown(sessionId);
        RuntimeRegistry.unregister(sessionId);

        // Clean up temporary files
        paths.cleanupTemp();

        if (lifecycleListener != null) {
            lifecycleListener.onDestroyed();
        }
    }

    /**
     * Alias for {@link #close()}.
     */
    public void destroy() {
        close();
    }

    // ==================== Input ====================

    /**
     * Dispatch a touch event to the game.
     *
     * @param event The MotionEvent from the view
     * @return true if the event was handled
     */
    public boolean dispatchTouchEvent(MotionEvent event) {
        if (destroyed || event == null) {
            return false;
        }
        touchHandler.dispatch(sessionId, event);
        return true;
    }

    // ==================== Callbacks ====================

    /**
     * Set a listener for game events.
     *
     * @param listener The listener, or null to remove
     */
    public void setOnGameEventListener(OnGameEventListener listener) {
        this.gameEventListener = listener;
    }

    /**
     * Set a listener for runtime errors.
     *
     * @param listener The listener, or null to remove
     */
    public void setOnErrorListener(OnErrorListener listener) {
        this.errorListener = listener;
    }

    /**
     * Set a listener for lifecycle events.
     *
     * @param listener The listener, or null to remove
     */
    public void setOnLifecycleListener(OnLifecycleListener listener) {
        this.lifecycleListener = listener;
    }

    // ==================== Internal callbacks from native ====================

    /** @hide Called from native code */
    void notifyGameReady() {
        if (gameEventListener != null) {
            gameEventListener.onGameReady();
        }
    }

    /** @hide Called from native code */
    void notifyGameExit(int exitCode) {
        if (gameEventListener != null) {
            gameEventListener.onGameExit(exitCode);
        }
    }

    /** @hide Called from native code */
    void notifyError(int errorCode, String message, boolean recoverable) {
        if (errorListener != null) {
            errorListener.onError(errorCode, message, recoverable);
        }
    }

    // ==================== Helpers ====================

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
