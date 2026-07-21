package com.example.migo.sample;

import android.app.Activity;
import android.os.Bundle;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

import com.migo.runtime.GameSession;
import com.migo.runtime.MigoRuntime;
import com.migo.runtime.RuntimeConfig;

/**
 * Minimal example - just 50 lines of code to run a game!
 * 
 * Game code should be deployed to session.getPaths().getCodeDir() before starting.
 */
public class MinimalActivity extends Activity implements SurfaceHolder.Callback {
    
    private GameSession session;
    private final String gameId = "my-game";       // Unique identifier for this game
    private final String entryPoint = "game.js";   // Entry point in the code directory

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        
        SurfaceView surface = new SurfaceView(this);
        surface.getHolder().addCallback(this);
        surface.setOnTouchListener((v, e) -> session != null && session.dispatchTouchEvent(e));
        setContentView(surface);
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        RuntimeConfig config = new RuntimeConfig.Builder(this).build();
        // gameId creates isolated directories for this game
        session = MigoRuntime.getInstance().createSession(this, holder.getSurface(), config, gameId);
        // Ensure game code is in session.getPaths().getCodeDir() before this call
        session.startGame(entryPoint);
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int f, int w, int h) {
        if (session != null) session.updateSurface(holder.getSurface(), w, h);
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        if (session != null) session.onSurfaceDestroyed();
    }

    @Override protected void onPause() { super.onPause(); if (session != null) session.pause(); }
    @Override protected void onResume() { super.onResume(); if (session != null) session.resume(); }
    @Override protected void onDestroy() { if (session != null) session.close(); super.onDestroy(); }
}
