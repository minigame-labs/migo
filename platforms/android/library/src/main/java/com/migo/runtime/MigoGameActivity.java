package com.migo.runtime;

import android.app.Activity;
import android.content.ComponentCallbacks2;
import android.content.Context;
import android.content.Intent;
import android.content.res.Configuration;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.widget.FrameLayout;

import com.migo.runtime.callback.GameSessionListener;

/**
 * Ready-to-use Activity for running a game with zero boilerplate.
 * <p>
 * All lifecycle management (pause, resume, surface, touch, memory warnings)
 * is handled internally. The host app only needs to launch this Activity.
 *
 * <h3>Usage</h3>
 * <pre>{@code
 * // Launch a game in one line:
 * MigoGameActivity.launch(context, "my-game", "game.js");
 *
 * // Or with custom config:
 * MigoGameActivity.launch(context, "my-game", "game.js",
 *     new RuntimeConfig.Builder(context)
 *         .setDebugEnabled(true)
 *         .build());
 * }</pre>
 *
 * <h3>Receiving game events</h3>
 * <p>
 * To receive game events, subclass this Activity and override
 * {@link #onCreateGameListener()}:
 * <pre>{@code
 * public class MyGameActivity extends MigoGameActivity {
 *     @Override
 *     protected GameSessionListener onCreateGameListener() {
 *         return new GameSessionListener() {
 *             @Override public void onGameReady() { ... }
 *             @Override public void onGameExit(int code) { finish(); }
 *             @Override public void onError(int code, String msg, boolean recoverable) { ... }
 *         };
 *     }
 * }
 * }</pre>
 *
 * @since 2.0.0
 */
public class MigoGameActivity extends Activity
        implements SurfaceHolder.Callback, ComponentCallbacks2 {

    private static final String TAG = "MigoGameActivity";

    /** Intent extra key for game ID. */
    public static final String EXTRA_GAME_ID = "migo_game_id";
    /** Intent extra key for entry point file name. */
    public static final String EXTRA_ENTRY_POINT = "migo_entry_point";

    private static RuntimeConfig sPendingConfig;
    private static final Object sConfigLock = new Object();

    private GameSession session;
    private SurfaceView surfaceView;
    private String gameId;
    private String entryPoint;
    private RuntimeConfig config;

    /**
     * Launch a game with default configuration.
     *
     * @param context    Context to start from
     * @param gameId     Unique game identifier
     * @param entryPoint Entry point file (e.g., "game.js")
     */
    public static void launch(Context context, String gameId, String entryPoint) {
        launch(context, gameId, entryPoint, null);
    }

    /**
     * Launch a game with custom configuration.
     *
     * @param context    Context to start from
     * @param gameId     Unique game identifier
     * @param entryPoint Entry point file (e.g., "game.js")
     * @param config     Runtime configuration (null for defaults)
     */
    public static void launch(Context context, String gameId, String entryPoint,
                              RuntimeConfig config) {
        synchronized (sConfigLock) {
            sPendingConfig = config;
        }
        Intent intent = new Intent(context, MigoGameActivity.class);
        intent.putExtra(EXTRA_GAME_ID, gameId);
        intent.putExtra(EXTRA_ENTRY_POINT, entryPoint);
        if (!(context instanceof Activity)) {
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        }
        context.startActivity(intent);
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Extract launch parameters
        gameId = getIntent().getStringExtra(EXTRA_GAME_ID);
        entryPoint = getIntent().getStringExtra(EXTRA_ENTRY_POINT);
        if (gameId == null || entryPoint == null) {
            finish();
            return;
        }

        synchronized (sConfigLock) {
            config = sPendingConfig;
            sPendingConfig = null;
        }
        if (config == null) {
            config = new RuntimeConfig.Builder(this).build();
        }

        // Check runtime support
        MigoRuntime runtime = MigoRuntime.getInstance();
        if (!runtime.isDeviceSupported()) {
            finish();
            return;
        }

        // Create surface view
        FrameLayout root = new FrameLayout(this);
        surfaceView = new SurfaceView(this);
        root.addView(surfaceView, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));
        setContentView(root);

        // Set up surface callbacks
        surfaceView.getHolder().addCallback(this);

        // Set up touch handling
        surfaceView.setOnTouchListener((v, event) -> {
            if (session != null && session.isValid()) {
                return session.dispatchTouchEvent(event);
            }
            return false;
        });

        // Register for memory warnings
        registerComponentCallbacks(this);
    }

    // ==================== SurfaceHolder.Callback ====================

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        Log.i(TAG, "surfaceCreated: frame=" + holder.getSurfaceFrame());
        if (session != null && session.isValid()) {
            session.updateSurface(holder.getSurface());
        } else {
            initializeGame(holder);
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        Log.i(TAG, "surfaceChanged: format=" + format + ", size=" + width + "x" + height);
        if (session != null && session.isValid()) {
            session.updateSurface(holder.getSurface());
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        Log.i(TAG, "surfaceDestroyed");
        // Do NOT destroy the session. Surface is destroyed on onStop but
        // Activity is still alive. Cleanup happens in onDestroy.
    }

    // ==================== Activity Lifecycle ====================

    @Override
    protected void onPause() {
        super.onPause();
        Log.i(TAG, "onPause");
        if (session != null && session.isValid()) {
            session.pause();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        Log.i(TAG, "onResume");
        if (session != null && session.isValid()) {
            session.resume();
        }
    }

    @Override
    protected void onDestroy() {
        unregisterComponentCallbacks(this);
        if (session != null) {
            session.close();
            session = null;
        }
        super.onDestroy();
    }

    // ==================== ComponentCallbacks2 ====================

    @Override
    public void onTrimMemory(int level) {
        if (session != null && session.isValid()) {
            session.dispatchMemoryWarning(level);
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {}

    @Override
    public void onLowMemory() {
        if (session != null && session.isValid()) {
            session.dispatchMemoryWarning(15); // TRIM_MEMORY_RUNNING_CRITICAL
        }
    }

    // ==================== Hooks for subclasses ====================

    /**
     * Override to provide a custom game session listener.
     * <p>
     * Called during game initialization. Return null for no listener.
     *
     * @return A GameSessionListener, or null
     */
    protected GameSessionListener onCreateGameListener() {
        return null;
    }

    /**
     * Get the current game session.
     *
     * @return The active GameSession, or null if not initialized
     */
    protected GameSession getGameSession() {
        return session;
    }

    // ==================== Private ====================

    private void initializeGame(SurfaceHolder holder) {
        Log.i(TAG, "initializeGame: gameId=" + gameId + ", entry=" + entryPoint);
        MigoRuntime.Result<GameSession> result = MigoRuntime.getInstance()
                .createSessionSafe(this, holder.getSurface(), config, gameId);

        if (result.isFailure()) {
            Log.e(TAG, "initializeGame failed: code=" + result.getErrorCode() + ", msg=" + result.getErrorMessage());
            finish();
            return;
        }

        session = result.getValue();

        // Set up listener
        GameSessionListener listener = onCreateGameListener();
        if (listener != null) {
            session.setListener(listener);
        }

        // Start the game
        int startResult = session.startGameSafe(entryPoint);
        Log.i(TAG, "startGameSafe result=" + startResult);
        if (startResult != ErrorCode.SUCCESS) {
            finish();
        }
    }
}
