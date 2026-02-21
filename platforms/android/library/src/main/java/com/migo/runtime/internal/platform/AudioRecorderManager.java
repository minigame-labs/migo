package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.media.MediaRecorder;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Platform-level audio recorder wrapping Android's {@link MediaRecorder}.
 * <p>
 * Manages recording lifecycle (start/pause/resume/stop), option parsing,
 * auto-stop timer, and event dispatch to the native layer.
 * <p>
 * Events are sent via {@link NativeMethods#onRecorderEvent} and
 * frame data via {@link NativeMethods#onRecorderFrameData}.
 *
 * @hide
 */
public final class AudioRecorderManager {

    private static final String TAG = "AudioRecorderManager";

    /** Recording states. */
    private static final int STATE_IDLE = 0;
    private static final int STATE_RECORDING = 1;
    private static final int STATE_PAUSED = 2;

    private final int sessionId;
    private final Activity activity;
    private final Handler mainHandler;

    private MediaRecorder recorder;
    private final AtomicInteger state = new AtomicInteger(STATE_IDLE);

    // Recording options
    private int maxDuration = 60000;
    private int sampleRate = 8000;
    private int numberOfChannels = 2;
    private int encodeBitRate = 48000;
    private String format = "aac";
    private String audioSource = "auto";
    private int frameSize = 0; // 0 = disabled

    // Recording state
    private String outputFilePath;
    private long recordStartTime;
    private long recordedDuration; // accumulated duration across pause/resume cycles
    private Runnable autoStopRunnable;

    // Frame data reader (when frameSize > 0, uses PCM pipe)
    private Thread frameReaderThread;
    private volatile boolean frameReaderRunning;

    public AudioRecorderManager(int sessionId, Activity activity) {
        this.sessionId = sessionId;
        this.activity = activity;
        this.mainHandler = new Handler(Looper.getMainLooper());
    }

    /**
     * Start recording with the given options JSON.
     *
     * @param optionsJson JSON string with keys: duration, sampleRate, numberOfChannels,
     *                    encodeBitRate, format, frameSize, audioSource
     */
    public void start(String optionsJson) {
        if (state.get() != STATE_IDLE) {
            // Already recording — stop first then restart
            stopInternal(false);
        }

        parseOptions(optionsJson);

        try {
            recorder = new MediaRecorder();

            // Audio source
            recorder.setAudioSource(mapAudioSource(audioSource));

            // Output format + encoder
            configureFormatAndEncoder(recorder);

            // Audio parameters
            recorder.setAudioSamplingRate(sampleRate);
            recorder.setAudioChannels(numberOfChannels);
            recorder.setAudioEncodingBitRate(encodeBitRate);

            // Output file
            outputFilePath = createOutputFile();
            recorder.setOutputFile(outputFilePath);

            // Max duration
            if (maxDuration > 0) {
                recorder.setMaxDuration(maxDuration);
            }

            recorder.setOnInfoListener((mr, what, extra) -> {
                if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED) {
                    mainHandler.post(() -> stopInternal(true));
                }
            });

            recorder.setOnErrorListener((mr, what, extra) -> {
                String errMsg = "MediaRecorder error: what=" + what + " extra=" + extra;
                fireEvent("error", "{\"errMsg\":\"" + escapeJson(errMsg) + "\"}");
                mainHandler.post(() -> resetRecorder());
            });

            recorder.prepare();
            recorder.start();

            state.set(STATE_RECORDING);
            recordStartTime = System.currentTimeMillis();
            recordedDuration = 0;

            // Schedule auto-stop
            scheduleAutoStop();

            fireEvent("start", "{}");
        } catch (Exception e) {
            fireEvent("error",
                    "{\"errMsg\":\"" + escapeJson("recorderManager.start:fail " + e.getMessage()) + "\"}");
            resetRecorder();
        }
    }

    /**
     * Pause recording (API 24+).
     */
    public void pause() {
        if (state.get() != STATE_RECORDING) return;

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            try {
                recorder.pause();
                state.set(STATE_PAUSED);
                // Accumulate duration
                recordedDuration += System.currentTimeMillis() - recordStartTime;
                cancelAutoStop();
                fireEvent("pause", "{}");
            } catch (Exception e) {
                fireEvent("error",
                        "{\"errMsg\":\"" + escapeJson("recorderManager.pause:fail " + e.getMessage()) + "\"}");
            }
        } else {
            fireEvent("error",
                    "{\"errMsg\":\"recorderManager.pause:fail not supported below API 24\"}");
        }
    }

    /**
     * Resume recording after pause (API 24+).
     */
    public void resume() {
        if (state.get() != STATE_PAUSED) return;

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            try {
                recorder.resume();
                state.set(STATE_RECORDING);
                recordStartTime = System.currentTimeMillis();
                scheduleAutoStop();
                fireEvent("resume", "{}");
            } catch (Exception e) {
                fireEvent("error",
                        "{\"errMsg\":\"" + escapeJson("recorderManager.resume:fail " + e.getMessage()) + "\"}");
            }
        } else {
            fireEvent("error",
                    "{\"errMsg\":\"recorderManager.resume:fail not supported below API 24\"}");
        }
    }

    /**
     * Stop recording.
     */
    public void stop() {
        stopInternal(true);
    }

    /**
     * Release all resources. Call on session destruction.
     */
    public void destroy() {
        stopInternal(false);
    }

    // ==================== Internal ====================

    private void stopInternal(boolean notifyStop) {
        int prevState = state.getAndSet(STATE_IDLE);
        if (prevState == STATE_IDLE) return;

        cancelAutoStop();
        stopFrameReader();

        // Accumulate final duration
        if (prevState == STATE_RECORDING) {
            recordedDuration += System.currentTimeMillis() - recordStartTime;
        }

        try {
            if (recorder != null) {
                recorder.stop();
            }
        } catch (Exception e) {
            // May throw if no data was recorded yet
        }

        resetRecorder();

        if (notifyStop && outputFilePath != null) {
            File file = new File(outputFilePath);
            long fileSize = file.exists() ? file.length() : 0;

            JSONObject result = new JSONObject();
            try {
                result.put("tempFilePath", outputFilePath);
                result.put("duration", recordedDuration);
                result.put("fileSize", fileSize);
            } catch (JSONException ignored) {}

            fireEvent("stop", result.toString());

            // If frameSize was set, send the file as final frame data
            if (frameSize > 0 && file.exists() && fileSize > 0) {
                sendFileAsFrameData(file);
            }
        }
    }

    private void resetRecorder() {
        if (recorder != null) {
            try {
                recorder.reset();
                recorder.release();
            } catch (Exception ignored) {}
            recorder = null;
        }
    }

    private void parseOptions(String json) {
        if (json == null || json.isEmpty()) return;
        try {
            JSONObject opts = new JSONObject(json);
            maxDuration = opts.optInt("duration", 60000);
            maxDuration = Math.max(0, Math.min(600000, maxDuration));
            sampleRate = opts.optInt("sampleRate", 8000);
            numberOfChannels = opts.optInt("numberOfChannels", 2);
            encodeBitRate = opts.optInt("encodeBitRate", 48000);
            format = opts.optString("format", "aac");
            audioSource = opts.optString("audioSource", "auto");
            frameSize = opts.optInt("frameSize", 0);
        } catch (JSONException e) {
            // Use defaults
        }
    }

    private int mapAudioSource(String source) {
        if (source == null) return MediaRecorder.AudioSource.DEFAULT;
        switch (source) {
            case "buildInMic":
            case "mic":
                return MediaRecorder.AudioSource.MIC;
            case "camcorder":
                return MediaRecorder.AudioSource.CAMCORDER;
            case "voice_recognition":
                return MediaRecorder.AudioSource.VOICE_RECOGNITION;
            case "voice_communication":
                return MediaRecorder.AudioSource.VOICE_COMMUNICATION;
            case "headsetMic":
                // Android doesn't have a separate headset mic source;
                // MIC will use headset if connected
                return MediaRecorder.AudioSource.MIC;
            case "auto":
            default:
                return MediaRecorder.AudioSource.DEFAULT;
        }
    }

    private void configureFormatAndEncoder(MediaRecorder recorder) {
        switch (format.toLowerCase()) {
            case "mp3":
                recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
                recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
                // Note: Android MediaRecorder doesn't natively support MP3 encoding.
                // Use AAC in MP4 container as closest equivalent.
                break;
            case "wav":
            case "pcm":
                // Android MediaRecorder doesn't support raw WAV/PCM output directly.
                // Use AMR_WB as fallback for WAV, or AAC in MPEG4.
                // For production, consider AudioRecord for raw PCM.
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    recorder.setOutputFormat(MediaRecorder.OutputFormat.OGG);
                    recorder.setAudioEncoder(MediaRecorder.AudioEncoder.OPUS);
                } else {
                    recorder.setOutputFormat(MediaRecorder.OutputFormat.THREE_GPP);
                    recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AMR_WB);
                }
                break;
            case "aac":
            default:
                recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
                recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
                break;
        }
    }

    private String createOutputFile() {
        File cacheDir = activity.getCacheDir();
        File recordDir = new File(cacheDir, "recordings");
        if (!recordDir.exists()) {
            recordDir.mkdirs();
        }

        String ext;
        switch (format.toLowerCase()) {
            case "mp3":
                ext = ".mp3";
                break;
            case "wav":
                ext = ".wav";
                break;
            case "pcm":
                ext = ".pcm";
                break;
            default:
                ext = ".m4a";
                break;
        }
        return new File(recordDir, "rec_" + sessionId + "_" + System.currentTimeMillis() + ext)
                .getAbsolutePath();
    }

    private void scheduleAutoStop() {
        cancelAutoStop();
        if (maxDuration <= 0) return;

        long remaining = maxDuration - recordedDuration;
        if (remaining <= 0) {
            mainHandler.post(() -> stopInternal(true));
            return;
        }

        autoStopRunnable = () -> stopInternal(true);
        mainHandler.postDelayed(autoStopRunnable, remaining);
    }

    private void cancelAutoStop() {
        if (autoStopRunnable != null) {
            mainHandler.removeCallbacks(autoStopRunnable);
            autoStopRunnable = null;
        }
    }

    private void stopFrameReader() {
        frameReaderRunning = false;
        if (frameReaderThread != null) {
            frameReaderThread.interrupt();
            frameReaderThread = null;
        }
    }

    /**
     * Read the recorded file and send as frame data (used when frameSize > 0).
     */
    private void sendFileAsFrameData(File file) {
        final int chunkSize = frameSize * 1024; // frameSize is in KB
        Thread thread = new Thread(() -> {
            try (FileInputStream fis = new FileInputStream(file)) {
                byte[] buffer = new byte[chunkSize];
                int bytesRead;
                while ((bytesRead = fis.read(buffer)) != -1) {
                    int available = fis.available();
                    boolean isLast = available == 0;
                    if (bytesRead < buffer.length) {
                        byte[] trimmed = new byte[bytesRead];
                        System.arraycopy(buffer, 0, trimmed, 0, bytesRead);
                        NativeMethods.onRecorderFrameData(sessionId, trimmed, isLast);
                    } else {
                        NativeMethods.onRecorderFrameData(sessionId, buffer.clone(), isLast);
                    }
                }
            } catch (IOException e) {
                fireEvent("error",
                        "{\"errMsg\":\"" + escapeJson("frame read error: " + e.getMessage()) + "\"}");
            }
        }, "RecorderFrameReader-" + sessionId);
        thread.setDaemon(true);
        thread.start();
    }

    private void fireEvent(String eventType, String jsonPayload) {
        NativeMethods.onRecorderEvent(sessionId, eventType, jsonPayload);
    }

    private static String escapeJson(String s) {
        if (s == null) return "";
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
}
