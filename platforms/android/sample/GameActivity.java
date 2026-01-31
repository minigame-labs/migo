package com.example.migo.sample;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

import com.migo.runtime.ErrorCode;
import com.migo.runtime.GameSession;
import com.migo.runtime.MigoRuntime;
import com.migo.runtime.RuntimeConfig;
import com.migo.runtime.callback.OnErrorListener;
import com.migo.runtime.callback.OnGameEventListener;
import com.migo.runtime.callback.OnLifecycleListener;

/**
 * Sample Activity demonstrating Migo Runtime integration.
 * <p>
 * This example shows:
 * - Creating and configuring the runtime
 * - Managing the game session lifecycle
 * - Handling touch events
 * - Using callbacks for game events and errors
 */
public class GameActivity extends Activity {
    
    private static final String TAG = "GameActivity";
    
    private GameSession session;
    private SurfaceView surfaceView;
    
    // Game configuration (adjust to your needs)
    // gameId is used to create isolated directories in: 
    //   files/migo/games/{gameId}/ and cache/migo/games/{gameId}/
    private static final String GAME_ID = "sample-game";
    private static final String GAME_ENTRY = "game.js";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        
        // Check if runtime is supported
        MigoRuntime runtime = MigoRuntime.getInstance();
        if (!runtime.isDeviceSupported()) {
            Log.e(TAG, "Device not supported");
            finish();
            return;
        }
        
        Log.i(TAG, "Migo Runtime v" + runtime.getVersion() + 
                   " (native: " + runtime.getNativeVersion() + ")");
        
        // Create surface view for rendering
        surfaceView = new SurfaceView(this);
        setContentView(surfaceView);
        
        // Set up surface callbacks
        surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder holder) {
                if (session != null && session.isValid()) {
                    // Surface recreated (e.g., returning from background).
                    // Re-attach the new surface so the render thread can recreate
                    // its EGL onscreen context.
                    session.updateSurface(holder.getSurface());
                    Log.d(TAG, "Surface recreated, updated session surface");
                } else {
                    // First-time initialization.
                    initializeGame(holder);
                }
            }

            @Override
            public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
                if (session != null && session.isValid()) {
                    session.updateSurface(holder.getSurface());
                    Log.d(TAG, "Surface changed: " + width + "x" + height);
                }
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder holder) {
                // Do NOT destroy the session when the surface is lost.
                // On Android, the surface is destroyed when the activity goes
                // to background (onStop). The session is paused via onPause()
                // and will be resumed when surfaceCreated is called again.
                // Full cleanup happens in onDestroy().
                Log.d(TAG, "Surface destroyed (session kept alive)");
            }
        });
        
        // Set up touch handling
        surfaceView.setOnTouchListener((v, event) -> {
            if (session != null && session.isValid()) {
                return session.dispatchTouchEvent(event);
            }
            return false;
        });
    }
    
    /**
     * Initialize the game session.
     */
    private void initializeGame(SurfaceHolder holder) {
        // Build configuration
        RuntimeConfig config = new RuntimeConfig.Builder(this)
            .setTargetFps(60)
            .setDebugEnabled(BuildConfig.DEBUG)
            .setLogLevel(BuildConfig.DEBUG ? 
                RuntimeConfig.LogLevel.DEBUG : 
                RuntimeConfig.LogLevel.WARN)
            .build();
        
        Log.d(TAG, "Config: " + config);
        
        // Create session (safe version with error handling)
        // The gameId is used to create isolated directories for this game
        MigoRuntime.Result<GameSession> result = MigoRuntime.getInstance()
            .createSessionSafe(this, holder.getSurface(), config, GAME_ID);
        
        if (result.isFailure()) {
            Log.e(TAG, "Failed to create session: " + 
                       ErrorCode.getMessage(result.getErrorCode()) +
                       " - " + result.getErrorMessage());
            finish();
            return;
        }
        
        session = result.getValue();
        
        // Set up callbacks
        setupCallbacks();
        
        // Start the game - path generation is handled by native layer
        // Game code should be deployed to session.getPaths().getCodeDir() before this call
        int startResult = session.startGameSafe(GAME_ENTRY);
        if (startResult != ErrorCode.SUCCESS) {
            Log.e(TAG, "Failed to start game: " + ErrorCode.getMessage(startResult));
            // Don't finish - let the error callback handle it
        }
    }
    
    /**
     * Set up game callbacks.
     */
    private void setupCallbacks() {
        // Game events
        session.setOnGameEventListener(new OnGameEventListener() {
            @Override
            public void onGameReady() {
                Log.i(TAG, "Game is ready!");
                runOnUiThread(() -> {
                    // Hide loading screen, start animations, etc.
                });
            }

            @Override
            public void onGameExit(int exitCode) {
                Log.i(TAG, "Game exited with code: " + exitCode);
                runOnUiThread(() -> {
                    if (exitCode == 0) {
                        finish();
                    } else {
                        // Show error or restart option
                    }
                });
            }

            @Override
            public void onLoadingStart() {
                Log.d(TAG, "Loading started");
                runOnUiThread(() -> {
                    // Show loading indicator
                });
            }

            @Override
            public void onLoadingEnd() {
                Log.d(TAG, "Loading ended");
                runOnUiThread(() -> {
                    // Hide loading indicator
                });
            }

            @Override
            public void onLoadingProgress(float progress, String message) {
                Log.d(TAG, "Loading: " + (int)(progress * 100) + "% - " + message);
                runOnUiThread(() -> {
                    // Update progress bar
                });
            }
        });
        
        // Error handling
        session.setOnErrorListener((errorCode, message, recoverable) -> {
            Log.e(TAG, "Error [" + errorCode + "]: " + message + 
                       " (recoverable: " + recoverable + ")");
            
            if (!recoverable) {
                runOnUiThread(() -> {
                    // Show error dialog and finish
                    showErrorAndFinish(message);
                });
            }
        });
        
        // Lifecycle events
        session.setOnLifecycleListener(new OnLifecycleListener() {
            @Override
            public void onInitialized() {
                Log.d(TAG, "Runtime initialized");
            }

            @Override
            public void onDestroyed() {
                Log.d(TAG, "Runtime destroyed");
            }

            @Override
            public void onPaused() {
                Log.d(TAG, "Runtime paused");
            }

            @Override
            public void onResumed() {
                Log.d(TAG, "Runtime resumed");
            }

            @Override
            public void onSurfaceChanged(int width, int height) {
                Log.d(TAG, "Surface: " + width + "x" + height);
            }
        });
    }
    
    /**
     * Clean up the game session.
     */
    private void destroyGame() {
        if (session != null) {
            session.close();
            session = null;
            Log.d(TAG, "Game session destroyed");
        }
    }
    
    /**
     * Show error dialog and finish activity.
     */
    private void showErrorAndFinish(String message) {
        new android.app.AlertDialog.Builder(this)
            .setTitle("Error")
            .setMessage(message)
            .setPositiveButton("OK", (d, w) -> finish())
            .setCancelable(false)
            .show();
    }
    
    // ==================== Activity Lifecycle ====================

    @Override
    protected void onPause() {
        super.onPause();
        if (session != null && session.isValid()) {
            session.pause();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (session != null && session.isValid()) {
            session.resume();
        }
    }

    @Override
    protected void onDestroy() {
        destroyGame();
        super.onDestroy();
    }
}
