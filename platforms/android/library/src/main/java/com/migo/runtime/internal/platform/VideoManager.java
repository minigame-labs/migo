package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.graphics.SurfaceTexture;
import android.media.MediaPlayer;
import android.os.Handler;
import android.os.Looper;
import android.view.Surface;
import android.view.TextureView;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONObject;

import java.lang.ref.WeakReference;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Manages video player instances for a session.
 * Each video instance uses a TextureView + MediaPlayer pair.
 * Events are dispatched back to JS via {@link NativeMethods#onVideoEvent}.
 *
 * @hide
 */
public class VideoManager {
    private final int sessionId;
    private final WeakReference<Activity> activityRef;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final Map<Integer, VideoPlayer> players = new ConcurrentHashMap<>();
    private final AtomicInteger nextVideoId = new AtomicInteger(1);

    public VideoManager(int sessionId, Activity activity) {
        this.sessionId = sessionId;
        this.activityRef = new WeakReference<>(activity);
    }

    private Activity getActivity() {
        return activityRef.get();
    }

    public String create(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            int videoId = nextVideoId.getAndIncrement();
            String src = opts.optString("src", "");
            // Store initial properties
            VideoPlayer player = new VideoPlayer(videoId, src, opts);
            players.put(videoId, player);

            // Create TextureView on UI thread
            Activity activity = getActivity();
            if (activity != null) {
                mainHandler.post(() -> player.createView(activity));
            }

            return "{\"videoId\":" + videoId + "}";
        } catch (Exception e) {
            throw new RuntimeException("videoCreate:fail " + e.getMessage());
        }
    }

    public void play(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) mainHandler.post(() -> p.play());
    }

    public void pause(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) mainHandler.post(() -> p.pause());
    }

    public void stop(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) mainHandler.post(() -> p.stop());
    }

    public void seek(String json) {
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            double position = obj.optDouble("position", 0);
            VideoPlayer p = players.get(videoId);
            if (p != null) mainHandler.post(() -> p.seek(position));
        } catch (Exception ignored) {}
    }

    public void requestFullscreen(String json) {
        // Fullscreen requires UI thread operations
        // Simplified: just notify JS of state change
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            NativeMethods.onVideoEvent(sessionId, videoId, "fullscreenchange", "{\"fullScreen\":true}");
        } catch (Exception ignored) {}
    }

    public void exitFullscreen(int videoId) {
        NativeMethods.onVideoEvent(sessionId, videoId, "fullscreenchange", "{\"fullScreen\":false}");
    }

    public void setProperty(String json) {
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            JSONObject props = obj.optJSONObject("properties");
            VideoPlayer p = players.get(videoId);
            if (p != null && props != null) {
                mainHandler.post(() -> p.setProperties(props));
            }
        } catch (Exception ignored) {}
    }

    public void destroyVideo(int videoId) {
        VideoPlayer p = players.remove(videoId);
        if (p != null) mainHandler.post(() -> p.release());
    }

    public void destroy() {
        for (VideoPlayer p : players.values()) {
            mainHandler.post(() -> p.release());
        }
        players.clear();
    }

    // Inner class: per-video-instance state
    private class VideoPlayer implements TextureView.SurfaceTextureListener,
            MediaPlayer.OnPreparedListener, MediaPlayer.OnCompletionListener,
            MediaPlayer.OnErrorListener, MediaPlayer.OnBufferingUpdateListener {

        final int videoId;
        String src;
        boolean looping;
        boolean muted;
        float playbackRate = 1.0f;
        MediaPlayer mediaPlayer;
        TextureView textureView;
        Surface surface;
        boolean prepared = false;
        boolean pendingPlay = false;
        Handler timeUpdateHandler;
        Runnable timeUpdateRunnable;

        VideoPlayer(int videoId, String src, JSONObject opts) {
            this.videoId = videoId;
            this.src = src;
            this.looping = opts.optBoolean("loop", false);
            this.muted = opts.optBoolean("muted", false);
            this.playbackRate = (float) opts.optDouble("playbackRate", 1.0);
        }

        void createView(Activity activity) {
            textureView = new TextureView(activity);
            textureView.setSurfaceTextureListener(this);
            // Add to activity's content view (overlay on game)
            ViewGroup root = activity.findViewById(android.R.id.content);
            if (root != null) {
                FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                );
                root.addView(textureView, lp);
                textureView.setVisibility(android.view.View.GONE);
            }
        }

        void play() {
            if (mediaPlayer == null) {
                initMediaPlayer();
            }
            if (prepared) {
                mediaPlayer.start();
                startTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "play", "{}");
            } else {
                pendingPlay = true;
            }
        }

        void pause() {
            if (mediaPlayer != null && mediaPlayer.isPlaying()) {
                mediaPlayer.pause();
                stopTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "pause", "{}");
            }
        }

        void stop() {
            if (mediaPlayer != null) {
                mediaPlayer.stop();
                prepared = false;
                stopTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "ended", "{}");
            }
        }

        void seek(double positionSec) {
            if (mediaPlayer != null && prepared) {
                mediaPlayer.seekTo((int) (positionSec * 1000));
            }
        }

        void setProperties(JSONObject props) {
            if (props.has("src")) {
                String newSrc = props.optString("src", "");
                if (!newSrc.equals(src)) {
                    src = newSrc;
                    if (mediaPlayer != null) {
                        mediaPlayer.reset();
                        prepared = false;
                        initMediaPlayer();
                    }
                }
            }
            if (props.has("muted")) {
                muted = props.optBoolean("muted", false);
                if (mediaPlayer != null) {
                    float vol = muted ? 0f : 1f;
                    mediaPlayer.setVolume(vol, vol);
                }
            }
            if (props.has("loop")) {
                looping = props.optBoolean("loop", false);
                if (mediaPlayer != null) mediaPlayer.setLooping(looping);
            }
            if (props.has("playbackRate")) {
                playbackRate = (float) props.optDouble("playbackRate", 1.0);
                if (mediaPlayer != null && android.os.Build.VERSION.SDK_INT >= 23) {
                    try {
                        mediaPlayer.setPlaybackParams(mediaPlayer.getPlaybackParams().setSpeed(playbackRate));
                    } catch (Exception ignored) {}
                }
            }
        }

        void release() {
            stopTimeUpdates();
            if (mediaPlayer != null) {
                try { mediaPlayer.release(); } catch (Exception ignored) {}
                mediaPlayer = null;
            }
            if (surface != null) {
                surface.release();
                surface = null;
            }
            if (textureView != null) {
                ViewGroup parent = (ViewGroup) textureView.getParent();
                if (parent != null) parent.removeView(textureView);
                textureView = null;
            }
        }

        private void initMediaPlayer() {
            try {
                mediaPlayer = new MediaPlayer();
                mediaPlayer.setDataSource(src);
                mediaPlayer.setOnPreparedListener(this);
                mediaPlayer.setOnCompletionListener(this);
                mediaPlayer.setOnErrorListener(this);
                mediaPlayer.setOnBufferingUpdateListener(this);
                if (surface != null) mediaPlayer.setSurface(surface);
                float vol = muted ? 0f : 1f;
                mediaPlayer.setVolume(vol, vol);
                mediaPlayer.setLooping(looping);
                mediaPlayer.prepareAsync();
            } catch (Exception e) {
                String msg = e.getMessage();
                if (msg == null) msg = "unknown error";
                NativeMethods.onVideoEvent(sessionId, videoId, "error",
                    "{\"errMsg\":\"" + msg.replace("\"", "\\\"") + "\"}");
            }
        }

        private void startTimeUpdates() {
            if (timeUpdateHandler == null) {
                timeUpdateHandler = new Handler(Looper.getMainLooper());
            }
            stopTimeUpdates();
            timeUpdateRunnable = new Runnable() {
                public void run() {
                    if (mediaPlayer != null && mediaPlayer.isPlaying()) {
                        int pos = mediaPlayer.getCurrentPosition();
                        int dur = mediaPlayer.getDuration();
                        NativeMethods.onVideoEvent(sessionId, videoId, "timeupdate",
                            "{\"position\":" + (pos / 1000.0) + ",\"duration\":" + (dur / 1000.0) + "}");
                        timeUpdateHandler.postDelayed(this, 250);
                    }
                }
            };
            timeUpdateHandler.postDelayed(timeUpdateRunnable, 250);
        }

        private void stopTimeUpdates() {
            if (timeUpdateHandler != null && timeUpdateRunnable != null) {
                timeUpdateHandler.removeCallbacks(timeUpdateRunnable);
            }
        }

        // TextureView callbacks
        public void onSurfaceTextureAvailable(SurfaceTexture st, int w, int h) {
            surface = new Surface(st);
            if (mediaPlayer != null) mediaPlayer.setSurface(surface);
            if (textureView != null) textureView.setVisibility(android.view.View.VISIBLE);
        }
        public void onSurfaceTextureSizeChanged(SurfaceTexture st, int w, int h) {}
        public boolean onSurfaceTextureDestroyed(SurfaceTexture st) {
            if (surface != null) { surface.release(); surface = null; }
            return true;
        }
        public void onSurfaceTextureUpdated(SurfaceTexture st) {}

        // MediaPlayer callbacks
        public void onPrepared(MediaPlayer mp) {
            prepared = true;
            if (android.os.Build.VERSION.SDK_INT >= 23 && playbackRate != 1.0f) {
                try { mp.setPlaybackParams(mp.getPlaybackParams().setSpeed(playbackRate)); } catch (Exception ignored) {}
            }
            if (pendingPlay) {
                pendingPlay = false;
                mp.start();
                startTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "play", "{}");
            }
        }
        public void onCompletion(MediaPlayer mp) {
            stopTimeUpdates();
            NativeMethods.onVideoEvent(sessionId, videoId, "ended", "{}");
        }
        public boolean onError(MediaPlayer mp, int what, int extra) {
            NativeMethods.onVideoEvent(sessionId, videoId, "error",
                "{\"errCode\":" + what + ",\"errMsg\":\"MediaPlayer error " + what + "/" + extra + "\"}");
            return true;
        }
        public void onBufferingUpdate(MediaPlayer mp, int percent) {
            if (prepared) {
                int dur = mp.getDuration();
                double buffered = dur > 0 ? (percent / 100.0) * (dur / 1000.0) : 0;
                NativeMethods.onVideoEvent(sessionId, videoId, "progress",
                    "{\"buffered\":" + buffered + "}");
            }
        }
    }
}
