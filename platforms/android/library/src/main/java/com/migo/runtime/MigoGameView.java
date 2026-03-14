package com.migo.runtime;

import android.app.Activity;
import android.app.Application;
import android.content.ComponentCallbacks2;
import android.content.Context;
import android.content.res.Configuration;
import android.os.Bundle;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import com.migo.runtime.callback.GameSessionListener;

/**
 * Self-contained game view that manages the full game lifecycle internally.
 * <p>
 * This is the recommended way to embed a game in your layout. It handles:
 * <ul>
 *   <li>Surface creation, change, and destruction</li>
 *   <li>Touch event dispatch</li>
 *   <li>Activity lifecycle (pause/resume/destroy) via ActivityLifecycleCallbacks</li>
 *   <li>Memory warning forwarding</li>
 *   <li>Debug overlay (when enabled)</li>
 * </ul>
 *
 * <h3>Usage</h3>
 * <pre>{@code
 * MigoGameView gameView = new MigoGameView(context);
 * RuntimeConfig config = new RuntimeConfig.Builder(context)
 *     .setImmersiveMode(false)  // usually false for embedded views
 *     .build();
 * gameView.setConfig(config);
 * gameView.setGameListener(listener);
 * layout.addView(gameView);
 * gameView.loadGame("my-game", "game.js");
 * }</pre>
 *
 * @since 2.0.0
 */
public class MigoGameView extends FrameLayout implements SurfaceHolder.Callback {

    private SurfaceView surfaceView;
    private GameSession session;
    private RuntimeConfig config;
    private GameSessionListener gameListener;
    private String pendingGameId;
    private String pendingEntryPoint;
    private boolean surfaceReady = false;
    private Activity boundActivity;
    private Application.ActivityLifecycleCallbacks lifecycleCallbacks;
    private ComponentCallbacks2 memoryCallback;

    public MigoGameView(Context context) {
        super(context);
        init();
    }

    private void init() {
        surfaceView = new SurfaceView(getContext());
        addView(surfaceView, new LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));
        surfaceView.getHolder().addCallback(this);
    }

    /**
     * Set the runtime configuration.
     * Must be called before {@link #loadGame}.
     *
     * @param config Runtime configuration
     */
    public void setConfig(RuntimeConfig config) {
        this.config = config;
    }

    /**
     * Set the game event listener.
     *
     * @param listener Listener for game events, or null to remove
     */
    public void setGameListener(GameSessionListener listener) {
        this.gameListener = listener;
        if (session != null) {
            session.setListener(listener);
        }
    }

    /**
     * Load and start a game.
     * <p>
     * If the surface is not ready yet, the game will start automatically
     * when the surface becomes available.
     *
     * @param gameId     Unique game identifier
     * @param entryPoint Entry point file (e.g., "game.js")
     */
    public void loadGame(String gameId, String entryPoint) {
        this.pendingGameId = gameId;
        this.pendingEntryPoint = entryPoint;

        if (surfaceReady) {
            startSession();
        }
    }

    /**
     * Get the current game session, if active.
     *
     * @return The active GameSession, or null
     */
    public GameSession getSession() {
        return session;
    }

    // ==================== SurfaceHolder.Callback ====================

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        surfaceReady = true;
        if (session != null && session.isValid()) {
            session.updateSurface(holder.getSurface());
        } else if (pendingGameId != null) {
            startSession();
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        if (session != null && session.isValid()) {
            session.updateSurface(holder.getSurface());
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        surfaceReady = false;
        // Do NOT close the session. The surface is recreated when the view
        // becomes visible again. Session cleanup happens in onDetachedFromWindow.
    }

    // ==================== Touch ====================

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        if (session != null && session.isValid()) {
            return session.dispatchTouchEvent(event);
        }
        return super.onTouchEvent(event);
    }

    // ==================== Lifecycle ====================

    @Override
    protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        boundActivity = findActivity(getContext());
        if (boundActivity != null) {
            registerLifecycleCallbacks(boundActivity);
        }
    }

    @Override
    protected void onDetachedFromWindow() {
        unregisterLifecycleCallbacks();
        destroySession();
        super.onDetachedFromWindow();
    }

    // ==================== Private ====================

    private void startSession() {
        if (session != null || pendingGameId == null) return;

        MigoRuntime runtime = MigoRuntime.getInstance();
        if (!runtime.isNativeLoaded()) return;

        RuntimeConfig cfg = config;
        if (cfg == null) {
            cfg = new RuntimeConfig.Builder(getContext()).build();
        }

        Activity activity = findActivity(getContext());
        if (activity != null) {
            session = runtime.createSession(activity,
                    surfaceView.getHolder().getSurface(), cfg, pendingGameId);
        } else {
            session = runtime.createSession(getContext(),
                    surfaceView.getHolder().getSurface(), cfg, pendingGameId);
        }

        if (gameListener != null) {
            session.setListener(gameListener);
        }

        // Add debug overlay if enabled
        DebugOverlayView overlay = session.getDebugOverlay();
        if (overlay != null) {
            LayoutParams lp = new LayoutParams(
                    LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT);
            lp.gravity = android.view.Gravity.TOP | android.view.Gravity.END;
            addView(overlay, lp);
        }

        session.startGame(pendingEntryPoint);
    }

    private void destroySession() {
        if (session != null) {
            session.close();
            session = null;
        }
    }

    private void registerLifecycleCallbacks(Activity activity) {
        lifecycleCallbacks = new EmptyActivityLifecycleCallbacks() {
            @Override
            public void onActivityPaused(Activity a) {
                if (a == activity && session != null && session.isValid()) {
                    session.pause();
                }
            }

            @Override
            public void onActivityResumed(Activity a) {
                if (a == activity && session != null && session.isValid()) {
                    session.resume();
                }
            }

            @Override
            public void onActivityDestroyed(Activity a) {
                if (a == activity) {
                    destroySession();
                }
            }
        };
        activity.getApplication().registerActivityLifecycleCallbacks(lifecycleCallbacks);

        memoryCallback = new ComponentCallbacks2() {
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
        };
        activity.registerComponentCallbacks(memoryCallback);
    }

    private void unregisterLifecycleCallbacks() {
        if (boundActivity != null && lifecycleCallbacks != null) {
            boundActivity.getApplication().unregisterActivityLifecycleCallbacks(lifecycleCallbacks);
            lifecycleCallbacks = null;
        }
        if (boundActivity != null && memoryCallback != null) {
            boundActivity.unregisterComponentCallbacks(memoryCallback);
            memoryCallback = null;
        }
        boundActivity = null;
    }

    private static Activity findActivity(Context context) {
        if (context instanceof Activity) {
            return (Activity) context;
        }
        if (context instanceof android.content.ContextWrapper) {
            return findActivity(((android.content.ContextWrapper) context).getBaseContext());
        }
        return null;
    }

    /**
     * Minimal implementation of ActivityLifecycleCallbacks with no-op defaults.
     */
    private static class EmptyActivityLifecycleCallbacks
            implements Application.ActivityLifecycleCallbacks {
        @Override public void onActivityCreated(Activity a, Bundle s) {}
        @Override public void onActivityStarted(Activity a) {}
        @Override public void onActivityResumed(Activity a) {}
        @Override public void onActivityPaused(Activity a) {}
        @Override public void onActivityStopped(Activity a) {}
        @Override public void onActivitySaveInstanceState(Activity a, Bundle s) {}
        @Override public void onActivityDestroyed(Activity a) {}
    }
}
