package com.migo.runtime;

import android.app.Activity;
import android.content.ComponentCallbacks2;
import android.content.Context;
import android.content.Intent;
import android.content.res.Configuration;
import android.os.Bundle;
import android.util.Log;
import android.graphics.Rect;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.widget.FrameLayout;

import com.migo.runtime.callback.GameSessionListener;
import com.migo.runtime.internal.NativeMethods;
import com.migo.runtime.internal.platform.DisplayCompat;
import com.migo.runtime.internal.platform.OrientationWaitHelper;

import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

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
 *             @Override public void onError(MigoException ex) { ... }
 *         };
 *     }
 * }
 * }</pre>
 *
 */
public class MigoGameActivity extends Activity
        implements SurfaceHolder.Callback, ComponentCallbacks2 {

    private static final String TAG = "MigoGameActivity";

    /** Intent extra key for game ID. */
    public static final String EXTRA_GAME_ID = "migo_game_id";
    /** Intent extra key for entry point file name. */
    public static final String EXTRA_ENTRY_POINT = "migo_entry_point";
    /** Internal extra key for pending RuntimeConfig token. */
    public static final String EXTRA_CONFIG_TOKEN = "migo_config_token";

    private static final ConcurrentHashMap<String, RuntimeConfig> sPendingConfigs =
            new ConcurrentHashMap<>();
    private static final ConcurrentHashMap<String, Long> sPendingConfigTimes =
            new ConcurrentHashMap<>();

    private GameSession session;
    private SurfaceView surfaceView;
    private final OrientationWaitHelper orientationHelper = new OrientationWaitHelper(TAG);
    private String lastOrientationEventValue;
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
        Intent intent = buildLaunchIntent(
                context,
                MigoGameActivity.class,
                gameId,
                entryPoint,
                config
        );
        context.startActivity(intent);
    }

    /**
     * Build a launch intent for MigoGameActivity (or subclasses).
     *
     * Subclasses can use this to launch themselves without relying on reflection.
     */
    protected static Intent buildLaunchIntent(
            Context context,
            Class<? extends MigoGameActivity> activityClass,
            String gameId,
            String entryPoint,
            RuntimeConfig config
    ) {
        Intent intent = new Intent(context, activityClass);
        intent.putExtra(EXTRA_GAME_ID, gameId);
        intent.putExtra(EXTRA_ENTRY_POINT, entryPoint);

        if (config != null) {
            // Clean up stale config tokens older than 30 seconds (e.g. Activity never started)
            long now = System.currentTimeMillis();
            sPendingConfigTimes.entrySet().removeIf(e -> {
                if (now - e.getValue() > 30_000) {
                    sPendingConfigs.remove(e.getKey());
                    return true;
                }
                return false;
            });

            String token = UUID.randomUUID().toString();
            sPendingConfigs.put(token, config);
            sPendingConfigTimes.put(token, now);
            intent.putExtra(EXTRA_CONFIG_TOKEN, token);
        }

        if (!(context instanceof Activity)) {
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        }
        return intent;
    }

    private static RuntimeConfig consumePendingConfig(Intent intent) {
        if (intent == null) {
            return null;
        }
        String token = intent.getStringExtra(EXTRA_CONFIG_TOKEN);
        if (token == null || token.isEmpty()) {
            return null;
        }
        sPendingConfigTimes.remove(token);
        return sPendingConfigs.remove(token);
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Extract launch parameters
        config = consumePendingConfig(getIntent());
        gameId = getIntent().getStringExtra(EXTRA_GAME_ID);
        entryPoint = getIntent().getStringExtra(EXTRA_ENTRY_POINT);
        if (gameId == null || entryPoint == null) {
            onLaunchFailed(ErrorCode.ERR_INVALID_GAME_ID, "Missing gameId or entryPoint");
            return;
        }
        if (config == null) {
            config = new RuntimeConfig.Builder(this).build();
        }

        // Check runtime support
        MigoRuntime runtime = MigoRuntime.getInstance();
        if (!runtime.isDeviceSupported()) {
            onLaunchFailed(ErrorCode.ERR_NOT_SUPPORTED, "Device not supported");
            return;
        }

        applyStartupOrientation();

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
            orientationHelper.cancel();
            Rect frame = holder.getSurfaceFrame();
            session.updateSurface(holder.getSurface(), frame.width(), frame.height());
        } else if (orientationHelper.getTargetOrientation() != null) {
            Rect frame = holder.getSurfaceFrame();
            if (orientationHelper.surfaceMatches(frame.width(), frame.height())) {
                initializeGame(holder);
            } else {
                Log.i(TAG, "surfaceCreated: deferring init until surface matches "
                        + orientationHelper.getTargetOrientation());
                orientationHelper.defer(holder, this::initializeGame);
            }
        } else {
            initializeGame(holder);
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        Log.i(TAG, "surfaceChanged: format=" + format + ", size=" + width + "x" + height);
        if (session != null && session.isValid()) {
            orientationHelper.cancel();
            session.updateSurface(holder.getSurface(), width, height);
        } else {
            SurfaceHolder pending = orientationHelper.consumePending();
            if (pending != null && orientationHelper.surfaceMatches(width, height)) {
                Log.i(TAG, "surfaceChanged: surface matches, initializing game");
                initializeGame(pending);
            } else if (pending != null) {
                orientationHelper.defer(pending, this::initializeGame);
            }
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        Log.i(TAG, "surfaceDestroyed");
        orientationHelper.cancel();
        if (session != null && session.isValid()) {
            session.onSurfaceDestroyed();
        }
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
        orientationHelper.cancel();
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
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        if (session != null && session.isValid()) {
            String value = DisplayCompat.mapDeviceOrientationValue(this, newConfig);
            if (!value.equals(lastOrientationEventValue)) {
                lastOrientationEventValue = value;
                NativeMethods.onDeviceOrientationChange(session.getSessionId(), value);
            }
        }
    }

    @Override
    public void onLowMemory() {
        if (session != null && session.isValid()) {
            session.dispatchMemoryWarning(15); // TRIM_MEMORY_RUNNING_CRITICAL
        }
    }

    // ==================== Hooks for subclasses ====================

    /**
     * Called when game launch fails. Override to show an error UI or report the failure.
     * Default implementation logs and finishes the activity.
     *
     * @param errorCode error code from {@link ErrorCode}
     * @param message   human-readable error description
     */
    protected void onLaunchFailed(int errorCode, String message) {
        Log.e(TAG, "Launch failed: [" + errorCode + "] " + message);
        finish();
    }

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

    /**
     * Called immediately after session creation and before startGame.
     *
     * Override this to register host handlers that should be available before
     * game code starts running.
     */
    protected void onSessionCreated(GameSession session) {
    }

    // ==================== Private ====================

    private void initializeGame(SurfaceHolder holder) {
        orientationHelper.cancel();
        Log.i(TAG, "initializeGame: gameId=" + gameId + ", entry=" + entryPoint);
        MigoRuntime.Result<GameSession> result = MigoRuntime.getInstance()
                .createSessionSafe(this, holder.getSurface(), config, gameId);

        if (result.isFailure()) {
            Log.e(TAG, "initializeGame failed: code=" + result.getErrorCode() + ", msg=" + result.getErrorMessage());
            onLaunchFailed(result.getErrorCode(), result.getErrorMessage());
            return;
        }

        session = result.getValue();
        lastOrientationEventValue = DisplayCompat.mapDeviceOrientationValue(
                this,
                getResources().getConfiguration()
        );

        try {
            onSessionCreated(session);
        } catch (Throwable t) {
            Log.e(TAG, "onSessionCreated failed", t);
            onLaunchFailed(ErrorCode.ERR_INIT_FAILED, "onSessionCreated threw: " + t.getMessage());
            return;
        }

        // Set up listener
        GameSessionListener listener = onCreateGameListener();
        if (listener != null) {
            session.setListener(listener);
        }

        // Start the game
        int startResult = session.startGameSafe(entryPoint);
        Log.i(TAG, "startGameSafe result=" + startResult);
        if (startResult != ErrorCode.SUCCESS) {
            onLaunchFailed(startResult, ErrorCode.getMessage(startResult));
        }
    }

    private void applyStartupOrientation() {
        String orientation = config != null ? config.getStartupOrientation() : null;
        if (orientation == null) {
            orientationHelper.setTargetOrientation(null);
            return;
        }

        orientationHelper.setTargetOrientation(orientation);
        int result = DisplayCompat.setDeviceOrientation(this, orientation);
        Log.i(TAG, "applyStartupOrientation: " + orientation + ", result=" + result);
    }
}
