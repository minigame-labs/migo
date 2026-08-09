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
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.atomic.AtomicBoolean;

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
    private final ConcurrentMap<Integer, VideoPlayer> players = new ConcurrentHashMap<>();
    private final AtomicBoolean lifecycleSuspendedTarget;
    private final AtomicBoolean destroyed = new AtomicBoolean(false);
    private boolean lifecycleSuspended;

    public VideoManager(int sessionId, Activity activity) {
        this(sessionId, activity, false);
    }

    public VideoManager(int sessionId, Activity activity, boolean lifecycleSuspended) {
        this.sessionId = sessionId;
        this.activityRef = new WeakReference<>(activity);
        this.lifecycleSuspendedTarget = new AtomicBoolean(lifecycleSuspended);
        this.lifecycleSuspended = lifecycleSuspended;
    }

    private Activity getActivity() {
        return activityRef.get();
    }

    private void runOnMain(Runnable action) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            action.run();
        } else {
            mainHandler.post(action);
        }
    }

    /**
     * The video id this create request carries.
     *
     * <p>The runtime allocates it, keys its own registry by it, and routes every
     * {@code onVideoEvent} through it, so Android has to honour it rather than
     * number its own. The counter this replaced started at 1 in both places and
     * agreed only while nothing ever failed on one side alone.
     *
     * <p>{@code optInt} is deliberately not used: it truncates, so {@code 1.5}
     * reads back as video 1 — an id that names a different, live player.
     */
    static int requestedVideoId(JSONObject opts) {
        Object raw = opts == null ? null : opts.opt("videoId");
        long id;
        if (raw instanceof Integer) {
            id = (Integer) raw;
        } else if (raw instanceof Long) {
            id = (Long) raw;
        } else {
            throw new IllegalArgumentException("videoId is missing or not an integer");
        }
        if (id <= 0 || id > Integer.MAX_VALUE) {
            throw new IllegalArgumentException("videoId " + id + " is outside 1..2147483647");
        }
        return (int) id;
    }

    /**
     * Registers {@code player} under {@code videoId}, refusing an id already live.
     *
     * <p>The map is the claim: {@code putIfAbsent} decides and inserts in one
     * step, so two creates racing for one id cannot both be told they have it.
     * A plain {@code Map} could not promise that, which is why the parameter
     * names the concurrent one.
     */
    static <T> void claimVideoId(ConcurrentMap<Integer, T> live, int videoId, T player) {
        if (live.putIfAbsent(videoId, player) != null) {
            throw new IllegalStateException("videoId " + videoId + " is already live");
        }
    }

    public String create(String optionsJson) {
        try {
            if (destroyed.get()) throw new IllegalStateException("session destroyed");
            JSONObject opts = new JSONObject(optionsJson);
            int videoId = requestedVideoId(opts);
            String src = opts.optString("src", "");
            // Store initial properties
            VideoPlayer player = new VideoPlayer(videoId, src, opts);
            claimVideoId(players, videoId, player);
            if (destroyed.get() && players.remove(videoId, player)) {
                runOnMain(player::release);
                throw new IllegalStateException("session destroyed");
            }

            // Create TextureView on UI thread
            Activity activity = getActivity();
            if (activity != null) {
                runOnMain(() -> player.createView(activity));
            }

            return "{\"videoId\":" + videoId + "}";
        } catch (Exception e) {
            throw new RuntimeException("videoCreate:fail " + e.getMessage());
        }
    }

    public void play(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) runOnMain(() -> p.play());
    }

    public void pause(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) runOnMain(() -> p.pause());
    }

    public void stop(int videoId) {
        VideoPlayer p = players.get(videoId);
        if (p != null) runOnMain(() -> p.stop());
    }

    public void seek(String json) {
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            double position = obj.optDouble("position", 0);
            VideoPlayer p = players.get(videoId);
            if (p != null) runOnMain(() -> p.seek(position));
        } catch (Exception ignored) {}
    }

    public void requestFullscreen(String json) {
        if (destroyed.get()) return;
        // Fullscreen requires UI thread operations
        // Simplified: just notify JS of state change
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            NativeMethods.onVideoEvent(sessionId, videoId, "fullscreenchange", "{\"fullScreen\":true}");
        } catch (Exception ignored) {}
    }

    public void exitFullscreen(int videoId) {
        if (destroyed.get()) return;
        NativeMethods.onVideoEvent(sessionId, videoId, "fullscreenchange", "{\"fullScreen\":false}");
    }

    public void setProperty(String json) {
        try {
            JSONObject obj = new JSONObject(json);
            int videoId = obj.optInt("videoId", 0);
            JSONObject props = obj.optJSONObject("properties");
            VideoPlayer p = players.get(videoId);
            if (p != null && props != null) {
                runOnMain(() -> p.setProperties(props));
            }
        } catch (Exception ignored) {}
    }

    public void destroyVideo(int videoId) {
        VideoPlayer p = players.remove(videoId);
        if (p != null) runOnMain(() -> p.release());
    }

    public void destroy() {
        if (!destroyed.compareAndSet(false, true)) return;
        runOnMain(() -> {
            for (VideoPlayer p : players.values()) {
                p.release();
            }
            players.clear();
        });
    }

    public synchronized void setLifecycleSuspended(boolean suspended) {
        if (destroyed.get()) return;
        lifecycleSuspendedTarget.set(suspended);
        runOnMain(this::applyLifecycleTarget);
    }

    public synchronized void suspendForLifecycle() {
        setLifecycleSuspended(true);
    }

    public synchronized void resumeForLifecycle() {
        setLifecycleSuspended(false);
    }

    private void applyLifecycleTarget() {
        if (destroyed.get()) return;
        boolean suspended = lifecycleSuspendedTarget.get();
        if (suspended) {
            if (lifecycleSuspended) return;
            lifecycleSuspended = true;
            for (VideoPlayer player : players.values()) {
                player.suspendForLifecycle();
            }
        } else {
            if (!lifecycleSuspended) return;
            lifecycleSuspended = false;
            for (VideoPlayer player : players.values()) {
                player.resumeForLifecycle();
            }
        }
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
        boolean resumeAfterLifecyclePause = false;
        final ResourceLifetime lifetime = new ResourceLifetime();
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
            if (!lifetime.canRun()) return;
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
            if (!lifetime.canRun()) return;
            resumeAfterLifecyclePause = false;
            if (lifecycleSuspended) {
                pendingPlay = true;
                return;
            }
            if (mediaPlayer == null) {
                initMediaPlayer();
            }
            if (prepared) {
                mediaPlayer.start();
                startTimeUpdates();
                pendingPlay = false;
                NativeMethods.onVideoEvent(sessionId, videoId, "play", "{}");
            } else {
                pendingPlay = true;
            }
        }

        void pause() {
            if (!lifetime.canRun()) return;
            pendingPlay = false;
            resumeAfterLifecyclePause = false;
            if (mediaPlayer != null && mediaPlayer.isPlaying()) {
                mediaPlayer.pause();
                stopTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "pause", "{}");
            }
        }

        void stop() {
            if (!lifetime.canRun()) return;
            pendingPlay = false;
            resumeAfterLifecyclePause = false;
            if (mediaPlayer != null) {
                mediaPlayer.stop();
                prepared = false;
                stopTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "ended", "{}");
            }
        }

        void seek(double positionSec) {
            if (!lifetime.canRun()) return;
            if (mediaPlayer != null && prepared) {
                mediaPlayer.seekTo((int) (positionSec * 1000));
            }
        }

        void setProperties(JSONObject props) {
            if (!lifetime.canRun()) return;
            if (props.has("src")) {
                String newSrc = props.optString("src", "");
                if (!newSrc.equals(src)) {
                    src = newSrc;
                    pendingPlay = false;
                    resumeAfterLifecyclePause = false;
                    if (mediaPlayer != null) {
                        if (lifecycleSuspended) {
                            try { mediaPlayer.release(); } catch (Exception ignored) {}
                            mediaPlayer = null;
                        } else {
                            mediaPlayer.reset();
                            initMediaPlayer();
                        }
                        prepared = false;
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
            if (!lifetime.release()) return;
            pendingPlay = false;
            resumeAfterLifecyclePause = false;
            prepared = false;
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

        void suspendForLifecycle() {
            if (!lifetime.canRun()) return;
            if (mediaPlayer == null || !prepared) return;
            try {
                if (mediaPlayer.isPlaying()) {
                    mediaPlayer.pause();
                    stopTimeUpdates();
                    resumeAfterLifecyclePause = true;
                }
            } catch (IllegalStateException ignored) {
                resumeAfterLifecyclePause = false;
            }
        }

        void resumeForLifecycle() {
            if (!lifetime.canRun()) return;
            if (resumeAfterLifecyclePause) {
                resumeAfterLifecyclePause = false;
                if (mediaPlayer != null && prepared) {
                    try {
                        mediaPlayer.start();
                        startTimeUpdates();
                    } catch (IllegalStateException ignored) {}
                }
                return;
            }
            if (pendingPlay) {
                play();
            }
        }

        private void initMediaPlayer() {
            if (!lifetime.canRun()) return;
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
            if (!lifetime.canRun()) return;
            if (timeUpdateHandler == null) {
                timeUpdateHandler = new Handler(Looper.getMainLooper());
            }
            stopTimeUpdates();
            timeUpdateRunnable = new Runnable() {
                public void run() {
                    if (lifetime.canRun()
                            && !lifecycleSuspended
                            && mediaPlayer != null
                            && mediaPlayer.isPlaying()) {
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
            if (!lifetime.canRun()) return;
            surface = new Surface(st);
            if (mediaPlayer != null) mediaPlayer.setSurface(surface);
            if (textureView != null) textureView.setVisibility(android.view.View.VISIBLE);
        }
        public void onSurfaceTextureSizeChanged(SurfaceTexture st, int w, int h) {}
        public boolean onSurfaceTextureDestroyed(SurfaceTexture st) {
            if (!lifetime.canRun()) return true;
            if (surface != null) { surface.release(); surface = null; }
            return true;
        }
        public void onSurfaceTextureUpdated(SurfaceTexture st) {}

        // MediaPlayer callbacks
        public void onPrepared(MediaPlayer mp) {
            if (!lifetime.canRun()) {
                try { mp.release(); } catch (Exception ignored) {}
                return;
            }
            prepared = true;
            if (android.os.Build.VERSION.SDK_INT >= 23 && playbackRate != 1.0f) {
                try { mp.setPlaybackParams(mp.getPlaybackParams().setSpeed(playbackRate)); } catch (Exception ignored) {}
            }
            if (pendingPlay && !lifecycleSuspended) {
                pendingPlay = false;
                mp.start();
                startTimeUpdates();
                NativeMethods.onVideoEvent(sessionId, videoId, "play", "{}");
            }
        }
        public void onCompletion(MediaPlayer mp) {
            if (!lifetime.canRun()) return;
            pendingPlay = false;
            resumeAfterLifecyclePause = false;
            stopTimeUpdates();
            NativeMethods.onVideoEvent(sessionId, videoId, "ended", "{}");
        }
        public boolean onError(MediaPlayer mp, int what, int extra) {
            if (!lifetime.canRun()) return true;
            NativeMethods.onVideoEvent(sessionId, videoId, "error",
                "{\"errCode\":" + what + ",\"errMsg\":\"MediaPlayer error " + what + "/" + extra + "\"}");
            return true;
        }
        public void onBufferingUpdate(MediaPlayer mp, int percent) {
            if (lifetime.canRun() && prepared && !lifecycleSuspended) {
                int dur = mp.getDuration();
                double buffered = dur > 0 ? (percent / 100.0) * (dur / 1000.0) : 0;
                NativeMethods.onVideoEvent(sessionId, videoId, "progress",
                    "{\"buffered\":" + buffered + "}");
            }
        }
    }
}
