package com.migo.runtime.internal.platform;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.ImageFormat;
import android.graphics.SurfaceTexture;
import android.hardware.camera2.CameraAccessException;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.TotalCaptureResult;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.media.MediaRecorder;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.Looper;
import android.util.Size;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Platform-level camera manager using Camera2 API.
 * <p>
 * Supports photo capture, video recording, zoom control, and real-time frame streaming.
 * Events are sent via {@link NativeMethods#onCameraEvent} and
 * frame data via {@link NativeMethods#onCameraFrameData}.
 *
 * @hide
 */
public final class CameraManager {

    private static final String TAG = "CameraManager";

    /** Camera states. */
    private static final int STATE_CLOSED = 0;
    private static final int STATE_OPENED = 1;
    private static final int STATE_RECORDING = 2;

    private final int sessionId;
    private final int cameraId;
    private final Activity activity;
    private final Handler mainHandler;

    // Camera2 objects
    private android.hardware.camera2.CameraManager cameraManager;
    private CameraDevice cameraDevice;
    private CameraCaptureSession captureSession;
    private CaptureRequest.Builder previewRequestBuilder;
    private String hardwareCameraId;

    // Background thread for camera operations
    private HandlerThread backgroundThread;
    private Handler backgroundHandler;

    // Frame streaming
    private ImageReader frameReader;
    private final AtomicBoolean frameListening = new AtomicBoolean(false);

    // Photo capture
    private ImageReader photoReader;

    // Video recording
    private MediaRecorder mediaRecorder;
    private String videoFilePath;
    private long recordStartTime;
    private Runnable recordTimeoutRunnable;

    // Semaphore to prevent concurrent open/close
    private final Semaphore cameraOpenCloseLock = new Semaphore(1);

    private final AtomicInteger state = new AtomicInteger(STATE_CLOSED);

    // Configuration
    private String position = "back";   // "back" or "front"
    private String flash = "auto";      // "auto", "on", "off", "torch"
    private String sizePreset = "medium"; // "small", "medium", "large"
    private float currentZoom = 1.0f;
    private float maxZoom = 1.0f;

    // Resolved sizes
    private Size previewSize;
    private Size photoSize;
    private Size videoSize;

    public CameraManager(int sessionId, int cameraId, Activity activity) {
        this.sessionId = sessionId;
        this.cameraId = cameraId;
        this.activity = activity;
        this.mainHandler = new Handler(Looper.getMainLooper());
    }

    /**
     * Create and open the camera with the given options.
     *
     * @param optionsJson JSON with keys: pos, flash, size
     * @return JSON result string: {"cameraId": <id>}
     */
    public String create(String optionsJson) {
        parseOptions(optionsJson);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                && activity.checkSelfPermission(Manifest.permission.CAMERA)
                != PackageManager.PERMISSION_GRANTED) {
            fireEvent("authCancel", "{}");
            return errorJson("createCamera:fail auth deny");
        }

        cameraManager = (android.hardware.camera2.CameraManager)
                activity.getSystemService(Context.CAMERA_SERVICE);
        if (cameraManager == null) {
            return errorJson("createCamera:fail camera service unavailable");
        }

        try {
            hardwareCameraId = findCameraId();
            if (hardwareCameraId == null) {
                return errorJson("createCamera:fail no camera found for position: " + position);
            }

            resolveSizes();
            startBackgroundThread();
            openCamera();

            JSONObject result = new JSONObject();
            result.put("cameraId", cameraId);
            return result.toString();
        } catch (Exception e) {
            return errorJson("createCamera:fail " + e.getMessage());
        }
    }

    /**
     * Destroy the camera and release all resources.
     */
    public void destroy() {
        state.set(STATE_CLOSED);
        closeCamera();
        stopBackgroundThread();
        fireEvent("stop", "{}");
    }

    /**
     * Take a photo.
     *
     * @param optionsJson JSON with keys: quality ("high", "normal", "low")
     * @return JSON result: {"tempImagePath": "<path>"} or error
     */
    public String takePhoto(String optionsJson) {
        if (state.get() == STATE_CLOSED || captureSession == null || cameraDevice == null) {
            return errorJson("camera.takePhoto:fail camera not ready");
        }

        String quality = "normal";
        try {
            JSONObject opts = new JSONObject(optionsJson);
            quality = opts.optString("quality", "normal");
        } catch (JSONException ignored) {}

        int jpegQuality;
        switch (quality) {
            case "high":  jpegQuality = 95; break;
            case "low":   jpegQuality = 60; break;
            default:      jpegQuality = 80; break;
        }

        try {
            // Create photo ImageReader
            if (photoReader != null) {
                photoReader.close();
            }
            photoReader = ImageReader.newInstance(
                    photoSize.getWidth(), photoSize.getHeight(),
                    ImageFormat.JPEG, 1);

            final String tempPath = createTempFilePath("photo", ".jpg");
            final int finalJpegQuality = jpegQuality;

            photoReader.setOnImageAvailableListener(reader -> {
                Image image = null;
                try {
                    image = reader.acquireLatestImage();
                    if (image != null) {
                        ByteBuffer buffer = image.getPlanes()[0].getBuffer();
                        byte[] bytes = new byte[buffer.remaining()];
                        buffer.get(bytes);

                        FileOutputStream fos = new FileOutputStream(tempPath);
                        fos.write(bytes);
                        fos.close();
                    }
                } catch (IOException e) {
                    // handled below
                } finally {
                    if (image != null) image.close();
                }
            }, backgroundHandler);

            CaptureRequest.Builder captureBuilder =
                    cameraDevice.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE);
            captureBuilder.addTarget(photoReader.getSurface());
            captureBuilder.set(CaptureRequest.JPEG_QUALITY, (byte) finalJpegQuality);
            applyFlashMode(captureBuilder);
            applyZoom(captureBuilder);

            captureSession.capture(captureBuilder.build(),
                    new CameraCaptureSession.CaptureCallback() {
                        @Override
                        public void onCaptureCompleted(CameraCaptureSession session,
                                                       CaptureRequest request,
                                                       TotalCaptureResult result) {
                            // Photo captured successfully
                        }
                    }, backgroundHandler);

            // Wait briefly for photo processing
            Thread.sleep(500);

            JSONObject result = new JSONObject();
            result.put("tempImagePath", tempPath);
            return result.toString();
        } catch (Exception e) {
            return errorJson("camera.takePhoto:fail " + e.getMessage());
        }
    }

    /**
     * Start video recording.
     *
     * @param optionsJson JSON with keys: timeout (seconds)
     * @return JSON result: {} on success
     */
    public String startRecord(String optionsJson) {
        if (state.get() != STATE_OPENED || cameraDevice == null) {
            return errorJson("camera.startRecord:fail camera not ready");
        }

        int timeoutSec = 0;
        try {
            JSONObject opts = new JSONObject(optionsJson);
            timeoutSec = opts.optInt("timeout", 0);
        } catch (JSONException ignored) {}

        try {
            closeSession();

            videoFilePath = createTempFilePath("video", ".mp4");
            setupMediaRecorder();

            // Create recording session with preview + mediaRecorder surfaces
            List<android.view.Surface> surfaces = new ArrayList<>();
            android.view.Surface recorderSurface = mediaRecorder.getSurface();
            surfaces.add(recorderSurface);

            if (frameListening.get() && frameReader != null) {
                surfaces.add(frameReader.getSurface());
            }

            previewRequestBuilder = cameraDevice.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
            previewRequestBuilder.addTarget(recorderSurface);

            if (frameListening.get() && frameReader != null) {
                previewRequestBuilder.addTarget(frameReader.getSurface());
            }

            applyFlashMode(previewRequestBuilder);
            applyZoom(previewRequestBuilder);

            cameraDevice.createCaptureSession(surfaces,
                    new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession session) {
                            captureSession = session;
                            try {
                                captureSession.setRepeatingRequest(
                                        previewRequestBuilder.build(), null, backgroundHandler);
                                mediaRecorder.start();
                                state.set(STATE_RECORDING);
                                recordStartTime = System.currentTimeMillis();
                            } catch (Exception e) {
                                fireEvent("error",
                                        "{\"errMsg\":\"" + escapeJson("camera.startRecord:fail " + e.getMessage()) + "\"}");
                            }
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession session) {
                            fireEvent("error",
                                    "{\"errMsg\":\"camera.startRecord:fail session config failed\"}");
                        }
                    }, backgroundHandler);

            // Schedule timeout
            if (timeoutSec > 0) {
                final int ts = timeoutSec;
                recordTimeoutRunnable = () -> {
                    fireEvent("timeoutCallback", "{}");
                    stopRecordInternal();
                };
                mainHandler.postDelayed(recordTimeoutRunnable, timeoutSec * 1000L);
            }

            return "{}";
        } catch (Exception e) {
            return errorJson("camera.startRecord:fail " + e.getMessage());
        }
    }

    /**
     * Stop video recording.
     *
     * @param optionsJson JSON with keys: compressed (boolean)
     * @return JSON result: {"tempThumbPath": "", "tempVideoPath": "<path>"}
     */
    public String stopRecord(String optionsJson) {
        if (state.get() != STATE_RECORDING) {
            return errorJson("camera.stopRecord:fail not recording");
        }

        return stopRecordInternal();
    }

    /**
     * Set zoom level.
     *
     * @param optionsJson JSON with keys: zoom (number)
     * @return JSON result: {"zoom": <actual_zoom>}
     */
    public String setZoom(String optionsJson) {
        if (state.get() == STATE_CLOSED) {
            return errorJson("camera.setZoom:fail camera not ready");
        }

        float zoom = 1.0f;
        try {
            JSONObject opts = new JSONObject(optionsJson);
            zoom = (float) opts.optDouble("zoom", 1.0);
        } catch (JSONException ignored) {}

        currentZoom = Math.max(1.0f, Math.min(zoom, maxZoom));

        if (previewRequestBuilder != null && captureSession != null) {
            try {
                applyZoom(previewRequestBuilder);
                captureSession.setRepeatingRequest(
                        previewRequestBuilder.build(), null, backgroundHandler);
            } catch (Exception e) {
                return errorJson("camera.setZoom:fail " + e.getMessage());
            }
        }

        try {
            JSONObject result = new JSONObject();
            result.put("zoom", currentZoom);
            return result.toString();
        } catch (JSONException e) {
            return "{\"zoom\":" + currentZoom + "}";
        }
    }

    /**
     * Start listening for camera frame changes.
     */
    public void listenFrameChange() {
        if (frameListening.getAndSet(true)) {
            return; // already listening
        }

        if (state.get() == STATE_CLOSED || cameraDevice == null) {
            return;
        }

        try {
            createFrameReader();
            restartPreviewSession();
        } catch (Exception e) {
            fireEvent("error",
                    "{\"errMsg\":\"" + escapeJson("camera.listenFrameChange:fail " + e.getMessage()) + "\"}");
        }
    }

    /**
     * Stop listening for camera frame changes.
     */
    public void closeFrameChange() {
        if (!frameListening.getAndSet(false)) {
            return;
        }

        try {
            restartPreviewSession();
        } catch (Exception e) {
            // Best effort
        }

        if (frameReader != null) {
            frameReader.close();
            frameReader = null;
        }
    }

    // ========================================================================
    // Camera2 internals
    // ========================================================================

    private void openCamera() throws CameraAccessException, InterruptedException {
        if (!cameraOpenCloseLock.tryAcquire(2500, TimeUnit.MILLISECONDS)) {
            throw new RuntimeException("Timeout waiting to acquire camera lock");
        }

        try {
            cameraManager.openCamera(hardwareCameraId, new CameraDevice.StateCallback() {
                @Override
                public void onOpened(CameraDevice camera) {
                    cameraOpenCloseLock.release();
                    cameraDevice = camera;
                    state.set(STATE_OPENED);

                    try {
                        if (frameListening.get()) {
                            createFrameReader();
                        }
                        startPreviewSession();
                    } catch (Exception e) {
                        fireEvent("error",
                                "{\"errMsg\":\"" + escapeJson("camera open:fail " + e.getMessage()) + "\"}");
                    }
                }

                @Override
                public void onDisconnected(CameraDevice camera) {
                    cameraOpenCloseLock.release();
                    camera.close();
                    cameraDevice = null;
                    state.set(STATE_CLOSED);
                    fireEvent("stop", "{}");
                }

                @Override
                public void onError(CameraDevice camera, int error) {
                    cameraOpenCloseLock.release();
                    camera.close();
                    cameraDevice = null;
                    state.set(STATE_CLOSED);
                    fireEvent("error", "{\"errMsg\":\"camera device error: " + error + "\"}");
                }
            }, backgroundHandler);
        } catch (SecurityException e) {
            cameraOpenCloseLock.release();
            fireEvent("authCancel", "{}");
            throw new CameraAccessException(CameraAccessException.CAMERA_ERROR,
                    "Camera permission denied");
        }
    }

    private void closeCamera() {
        try {
            cameraOpenCloseLock.tryAcquire(2500, TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            // proceed anyway
        }

        try {
            cancelRecordTimeout();

            if (mediaRecorder != null) {
                try {
                    if (state.get() == STATE_RECORDING) {
                        mediaRecorder.stop();
                    }
                } catch (Exception ignored) {}
                mediaRecorder.reset();
                mediaRecorder.release();
                mediaRecorder = null;
            }

            closeSession();

            if (cameraDevice != null) {
                cameraDevice.close();
                cameraDevice = null;
            }

            if (frameReader != null) {
                frameReader.close();
                frameReader = null;
            }

            if (photoReader != null) {
                photoReader.close();
                photoReader = null;
            }
        } finally {
            cameraOpenCloseLock.release();
        }
    }

    private void closeSession() {
        if (captureSession != null) {
            try {
                captureSession.close();
            } catch (Exception ignored) {}
            captureSession = null;
        }
    }

    private void startPreviewSession() throws CameraAccessException {
        if (cameraDevice == null) return;

        List<android.view.Surface> surfaces = new ArrayList<>();

        // Use a dummy SurfaceTexture for preview (we don't render preview on screen)
        SurfaceTexture texture = new SurfaceTexture(0);
        texture.setDefaultBufferSize(previewSize.getWidth(), previewSize.getHeight());
        android.view.Surface previewSurface = new android.view.Surface(texture);
        surfaces.add(previewSurface);

        previewRequestBuilder = cameraDevice.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
        previewRequestBuilder.addTarget(previewSurface);

        if (frameListening.get() && frameReader != null) {
            surfaces.add(frameReader.getSurface());
            previewRequestBuilder.addTarget(frameReader.getSurface());
        }

        applyFlashMode(previewRequestBuilder);
        applyZoom(previewRequestBuilder);

        cameraDevice.createCaptureSession(surfaces,
                new CameraCaptureSession.StateCallback() {
                    @Override
                    public void onConfigured(CameraCaptureSession session) {
                        if (cameraDevice == null) return;
                        captureSession = session;
                        try {
                            captureSession.setRepeatingRequest(
                                    previewRequestBuilder.build(), null, backgroundHandler);
                        } catch (Exception e) {
                            fireEvent("error",
                                    "{\"errMsg\":\"" + escapeJson("preview:fail " + e.getMessage()) + "\"}");
                        }
                    }

                    @Override
                    public void onConfigureFailed(CameraCaptureSession session) {
                        fireEvent("error", "{\"errMsg\":\"preview session config failed\"}");
                    }
                }, backgroundHandler);
    }

    private void restartPreviewSession() throws CameraAccessException {
        if (state.get() == STATE_RECORDING) {
            // Don't restart preview during recording
            return;
        }
        closeSession();
        startPreviewSession();
    }

    private void createFrameReader() {
        if (frameReader != null) {
            frameReader.close();
        }

        // Use YUV_420_888 for frame data - widely supported and efficient
        frameReader = ImageReader.newInstance(
                previewSize.getWidth(), previewSize.getHeight(),
                ImageFormat.YUV_420_888, 2);

        frameReader.setOnImageAvailableListener(reader -> {
            if (!frameListening.get()) return;

            Image image = null;
            try {
                image = reader.acquireLatestImage();
                if (image == null) return;

                // Extract Y plane data (grayscale) for minimal overhead,
                // or full RGBA if needed. We send raw bytes for JS to process.
                ByteBuffer yBuffer = image.getPlanes()[0].getBuffer();
                ByteBuffer uBuffer = image.getPlanes()[1].getBuffer();
                ByteBuffer vBuffer = image.getPlanes()[2].getBuffer();

                int ySize = yBuffer.remaining();
                int uSize = uBuffer.remaining();
                int vSize = vBuffer.remaining();

                byte[] data = new byte[ySize + uSize + vSize];
                yBuffer.get(data, 0, ySize);
                uBuffer.get(data, ySize, uSize);
                vBuffer.get(data, ySize + uSize, vSize);

                NativeMethods.onCameraFrameData(sessionId, cameraId, data,
                        image.getWidth(), image.getHeight());
            } finally {
                if (image != null) image.close();
            }
        }, backgroundHandler);
    }

    // ========================================================================
    // Video recording internals
    // ========================================================================

    private void setupMediaRecorder() throws IOException {
        mediaRecorder = new MediaRecorder();
        mediaRecorder.setAudioSource(MediaRecorder.AudioSource.MIC);
        mediaRecorder.setVideoSource(MediaRecorder.VideoSource.SURFACE);
        mediaRecorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
        mediaRecorder.setOutputFile(videoFilePath);
        mediaRecorder.setVideoEncodingBitRate(10_000_000);
        mediaRecorder.setVideoFrameRate(30);
        mediaRecorder.setVideoSize(videoSize.getWidth(), videoSize.getHeight());
        mediaRecorder.setVideoEncoder(MediaRecorder.VideoEncoder.H264);
        mediaRecorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);

        // Handle front camera orientation
        if ("front".equals(position)) {
            mediaRecorder.setOrientationHint(270);
        } else {
            mediaRecorder.setOrientationHint(90);
        }

        mediaRecorder.prepare();
    }

    private String stopRecordInternal() {
        if (state.get() != STATE_RECORDING) {
            return errorJson("camera.stopRecord:fail not recording");
        }

        cancelRecordTimeout();
        state.set(STATE_OPENED);

        try {
            captureSession.stopRepeating();
        } catch (Exception ignored) {}

        try {
            mediaRecorder.stop();
        } catch (Exception ignored) {}

        mediaRecorder.reset();
        mediaRecorder.release();
        mediaRecorder = null;

        // Restart preview
        try {
            startPreviewSession();
        } catch (Exception e) {
            // Best effort
        }

        try {
            JSONObject result = new JSONObject();
            result.put("tempThumbPath", "");
            result.put("tempVideoPath", videoFilePath);
            return result.toString();
        } catch (JSONException e) {
            return "{\"tempThumbPath\":\"\",\"tempVideoPath\":\"" + escapeJson(videoFilePath) + "\"}";
        }
    }

    private void cancelRecordTimeout() {
        if (recordTimeoutRunnable != null) {
            mainHandler.removeCallbacks(recordTimeoutRunnable);
            recordTimeoutRunnable = null;
        }
    }

    // ========================================================================
    // Camera configuration helpers
    // ========================================================================

    private void parseOptions(String json) {
        if (json == null || json.isEmpty()) return;
        try {
            JSONObject opts = new JSONObject(json);
            position = opts.optString("pos", "back");
            flash = opts.optString("flash", "auto");
            sizePreset = opts.optString("size", "medium");
        } catch (JSONException ignored) {}
    }

    private String findCameraId() throws CameraAccessException {
        int targetFacing = "front".equals(position)
                ? CameraCharacteristics.LENS_FACING_FRONT
                : CameraCharacteristics.LENS_FACING_BACK;

        for (String id : cameraManager.getCameraIdList()) {
            CameraCharacteristics chars = cameraManager.getCameraCharacteristics(id);
            Integer facing = chars.get(CameraCharacteristics.LENS_FACING);
            if (facing != null && facing == targetFacing) {
                return id;
            }
        }
        return null;
    }

    private void resolveSizes() throws CameraAccessException {
        CameraCharacteristics chars =
                cameraManager.getCameraCharacteristics(hardwareCameraId);

        // Get max zoom
        Float maxDigitalZoom = chars.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM);
        maxZoom = maxDigitalZoom != null ? maxDigitalZoom : 1.0f;

        StreamConfigurationMap map = chars.get(
                CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
        if (map == null) {
            throw new CameraAccessException(CameraAccessException.CAMERA_ERROR,
                    "No stream configuration map");
        }

        // Determine target sizes based on preset
        Size[] jpegSizes = map.getOutputSizes(ImageFormat.JPEG);
        Size[] yuvSizes = map.getOutputSizes(ImageFormat.YUV_420_888);

        photoSize = choosePhotoSize(jpegSizes);
        previewSize = choosePreviewSize(yuvSizes);
        videoSize = chooseVideoSize(map.getOutputSizes(MediaRecorder.class));
    }

    private Size choosePhotoSize(Size[] sizes) {
        if (sizes == null || sizes.length == 0) {
            return new Size(1920, 1080);
        }

        int targetPixels;
        switch (sizePreset) {
            case "small":  targetPixels = 640 * 480; break;
            case "large":  targetPixels = 3840 * 2160; break;
            default:       targetPixels = 1920 * 1080; break;
        }

        return chooseClosestSize(sizes, targetPixels);
    }

    private Size choosePreviewSize(Size[] sizes) {
        if (sizes == null || sizes.length == 0) {
            return new Size(1280, 720);
        }

        // For frame streaming, use moderate resolution for performance
        int targetPixels;
        switch (sizePreset) {
            case "small":  targetPixels = 320 * 240; break;
            case "large":  targetPixels = 1920 * 1080; break;
            default:       targetPixels = 1280 * 720; break;
        }

        return chooseClosestSize(sizes, targetPixels);
    }

    private Size chooseVideoSize(Size[] sizes) {
        if (sizes == null || sizes.length == 0) {
            return new Size(1920, 1080);
        }

        // Prefer 1080p for video
        int targetPixels = 1920 * 1080;
        return chooseClosestSize(sizes, targetPixels);
    }

    private Size chooseClosestSize(Size[] sizes, int targetPixels) {
        Size best = sizes[0];
        int bestDiff = Math.abs(best.getWidth() * best.getHeight() - targetPixels);

        for (Size size : sizes) {
            int diff = Math.abs(size.getWidth() * size.getHeight() - targetPixels);
            if (diff < bestDiff) {
                best = size;
                bestDiff = diff;
            }
        }
        return best;
    }

    private void applyFlashMode(CaptureRequest.Builder builder) {
        switch (flash) {
            case "on":
                builder.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_SINGLE);
                builder.set(CaptureRequest.CONTROL_AE_MODE,
                        CaptureRequest.CONTROL_AE_MODE_ON_ALWAYS_FLASH);
                break;
            case "off":
                builder.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_OFF);
                builder.set(CaptureRequest.CONTROL_AE_MODE,
                        CaptureRequest.CONTROL_AE_MODE_ON);
                break;
            case "torch":
                builder.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_TORCH);
                builder.set(CaptureRequest.CONTROL_AE_MODE,
                        CaptureRequest.CONTROL_AE_MODE_ON);
                break;
            case "auto":
            default:
                builder.set(CaptureRequest.CONTROL_AE_MODE,
                        CaptureRequest.CONTROL_AE_MODE_ON_AUTO_FLASH);
                break;
        }
    }

    private void applyZoom(CaptureRequest.Builder builder) {
        if (currentZoom <= 1.0f) return;

        try {
            CameraCharacteristics chars =
                    cameraManager.getCameraCharacteristics(hardwareCameraId);
            android.graphics.Rect sensorRect =
                    chars.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE);
            if (sensorRect == null) return;

            float clampedZoom = Math.min(currentZoom, maxZoom);
            int cropW = (int) (sensorRect.width() / clampedZoom);
            int cropH = (int) (sensorRect.height() / clampedZoom);
            int left = (sensorRect.width() - cropW) / 2;
            int top = (sensorRect.height() - cropH) / 2;

            android.graphics.Rect cropRegion =
                    new android.graphics.Rect(left, top, left + cropW, top + cropH);
            builder.set(CaptureRequest.SCALER_CROP_REGION, cropRegion);
        } catch (Exception ignored) {}
    }

    // ========================================================================
    // Thread management
    // ========================================================================

    private void startBackgroundThread() {
        backgroundThread = new HandlerThread("CameraBackground-" + cameraId);
        backgroundThread.start();
        backgroundHandler = new Handler(backgroundThread.getLooper());
    }

    private void stopBackgroundThread() {
        if (backgroundThread != null) {
            backgroundThread.quitSafely();
            try {
                backgroundThread.join(1000);
            } catch (InterruptedException ignored) {}
            backgroundThread = null;
            backgroundHandler = null;
        }
    }

    // ========================================================================
    // Utility
    // ========================================================================

    private String createTempFilePath(String prefix, String extension) {
        File cacheDir = activity.getCacheDir();
        File cameraDir = new File(cacheDir, "camera");
        if (!cameraDir.exists()) {
            cameraDir.mkdirs();
        }
        return new File(cameraDir,
                prefix + "_" + cameraId + "_" + System.currentTimeMillis() + extension)
                .getAbsolutePath();
    }

    private void fireEvent(String eventType, String jsonPayload) {
        NativeMethods.onCameraEvent(sessionId, cameraId, eventType, jsonPayload);
    }

    private static String errorJson(String errMsg) {
        return "{\"_error\":{\"errMsg\":\"" + escapeJson(errMsg) + "\"}}";
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
