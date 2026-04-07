package com.migo.runtime.internal;

import android.app.Activity;

import com.migo.runtime.internal.platform.AudioRecorderManager;
import com.migo.runtime.internal.platform.CameraManager;
import com.migo.runtime.internal.platform.ImageApiManager;
import com.migo.runtime.internal.platform.VideoManager;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Domain class for per-session media manager delegation (recorder, camera, image API, video).
 *
 * @hide
 */
public final class MediaExports {

    private MediaExports() {}

    private static final Object sRecorderLock = new Object();
    private static final Object sCameraLock = new Object();
    private static final Object sImageApiLock = new Object();
    private static final Object sVideoLock = new Object();

    private static final ConcurrentHashMap<Integer, AudioRecorderManager> sRecorderManagers =
            new ConcurrentHashMap<>();

    /** Per-session camera managers, keyed by "sessionId:cameraId". */
    private static final ConcurrentHashMap<String, CameraManager> sCameraManagers =
            new ConcurrentHashMap<>();

    private static final ConcurrentHashMap<Integer, ImageApiManager> sImageApiManagers =
            new ConcurrentHashMap<>();

    private static final ConcurrentHashMap<Integer, VideoManager> sVideoManagers =
            new ConcurrentHashMap<>();

    // ==================== Recorder ====================

    public static void recorderStart(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            NativeMethods.onRecorderEvent(sessionId, "error",
                    "{\"errMsg\":\"recorderManager.start:fail no context\"}");
            return;
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            NativeMethods.onRecorderEvent(sessionId, "error",
                    "{\"errMsg\":\"recorderManager.start:fail no activity\"}");
            return;
        }

        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr == null) {
            synchronized (sRecorderLock) {
                mgr = sRecorderManagers.get(sessionId);
                if (mgr == null) {
                    mgr = new AudioRecorderManager(sessionId, activity);
                    sRecorderManagers.put(sessionId, mgr);
                }
            }
        }
        mgr.start(optionsJson);
    }

    public static void recorderPause(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.pause();
        }
    }

    public static void recorderResume(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.resume();
        }
    }

    public static void recorderStop(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.get(sessionId);
        if (mgr != null) {
            mgr.stop();
        }
    }

    public static void destroyRecorderManager(int sessionId) {
        AudioRecorderManager mgr = sRecorderManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Camera ====================

    private static String cameraKey(int sessionId, int cameraId) {
        return sessionId + ":" + cameraId;
    }

    private static int extractCameraId(String optionsJson) {
        if (optionsJson == null) return 0;
        try {
            org.json.JSONObject opts = new org.json.JSONObject(optionsJson);
            return opts.optInt("cameraId", 0);
        } catch (Exception e) {
            return 0;
        }
    }

    public static String cameraCreate(int sessionId, String optionsJson) {
        RuntimeContext ctx = RuntimeRegistry.get(sessionId);
        if (ctx == null) {
            return "{\"_error\":{\"errMsg\":\"createCamera:fail no context\"}}";
        }
        Activity activity = ctx.getActivity();
        if (activity == null) {
            return "{\"_error\":{\"errMsg\":\"createCamera:fail no activity\"}}";
        }

        int cameraId = 0;
        try {
            org.json.JSONObject opts = new org.json.JSONObject(optionsJson);
            cameraId = opts.optInt("cameraId", 0);
        } catch (Exception ignored) {}

        String key = cameraKey(sessionId, cameraId);
        CameraManager existing = sCameraManagers.get(key);
        if (existing != null) {
            existing.destroy();
            sCameraManagers.remove(key);
        }

        CameraManager mgr = new CameraManager(sessionId, cameraId, activity);
        String result = mgr.create(optionsJson);
        sCameraManagers.put(key, mgr);
        return result;
    }

    public static void cameraDestroy(int sessionId, int cameraId) {
        String key = cameraKey(sessionId, cameraId);
        CameraManager mgr = sCameraManagers.remove(key);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    public static String cameraTakePhoto(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.takePhoto:fail camera not found\"}}";
        }
        return mgr.takePhoto(optionsJson);
    }

    public static String cameraStartRecord(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.startRecord:fail camera not found\"}}";
        }
        return mgr.startRecord(optionsJson);
    }

    public static String cameraStopRecord(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.stopRecord:fail camera not found\"}}";
        }
        return mgr.stopRecord(optionsJson);
    }

    public static String cameraSetZoom(int sessionId, String optionsJson) {
        int cameraId = extractCameraId(optionsJson);
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr == null) {
            return "{\"_error\":{\"errMsg\":\"camera.setZoom:fail camera not found\"}}";
        }
        return mgr.setZoom(optionsJson);
    }

    public static void cameraListenFrameChange(int sessionId, int cameraId) {
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr != null) {
            mgr.listenFrameChange();
        }
    }

    public static void cameraCloseFrameChange(int sessionId, int cameraId) {
        CameraManager mgr = sCameraManagers.get(cameraKey(sessionId, cameraId));
        if (mgr != null) {
            mgr.closeFrameChange();
        }
    }

    public static void destroyCameraManagers(int sessionId) {
        String prefix = sessionId + ":";
        for (String key : sCameraManagers.keySet()) {
            if (key.startsWith(prefix)) {
                CameraManager mgr = sCameraManagers.remove(key);
                if (mgr != null) {
                    mgr.destroy();
                }
            }
        }
    }

    // ==================== Image API ====================

    private static ImageApiManager getOrCreateImageApiManager(int sessionId) {
        ImageApiManager existing = sImageApiManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sImageApiLock) {
            existing = sImageApiManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            ImageApiManager mgr = new ImageApiManager(sessionId, activity);
            sImageApiManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void imageSaveToPhotosAlbum(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;
        mgr.saveToPhotosAlbum(optionsJson);
    }

    public static void imagePreviewMedia(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;
        mgr.previewMedia(optionsJson);
    }

    public static void imagePreviewImage(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;
        mgr.previewImage(optionsJson);
    }

    public static void imageCompress(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) {
            NativeMethods.onCompressImageResult(sessionId,
                    "{\"error\":\"compressImage:fail no context\"}");
            return;
        }
        mgr.compressAsync(sessionId, optionsJson);
    }

    public static void imageChooseMessageFile(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;
        mgr.chooseMessageFile(optionsJson);
    }

    public static void imageChooseImage(int sessionId, String optionsJson) {
        ImageApiManager mgr = getOrCreateImageApiManager(sessionId);
        if (mgr == null) return;
        mgr.chooseImage(optionsJson);
    }

    public static void destroyImageApiManager(int sessionId) {
        ImageApiManager mgr = sImageApiManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Video ====================

    private static VideoManager getOrCreateVideoManager(int sessionId) {
        VideoManager existing = sVideoManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sVideoLock) {
            existing = sVideoManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            VideoManager mgr = new VideoManager(sessionId, activity);
            sVideoManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static String videoCreate(int sessionId, String optionsJson) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) throw new RuntimeException("videoCreate:fail no context");
        return mgr.create(optionsJson);
    }

    public static void videoPlay(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.play(videoId);
    }

    public static void videoPause(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.pause(videoId);
    }

    public static void videoStop(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.stop(videoId);
    }

    public static void videoSeek(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.seek(json);
    }

    public static void videoRequestFullscreen(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.requestFullscreen(json);
    }

    public static void videoExitFullscreen(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.exitFullscreen(videoId);
    }

    public static void videoSetProperty(int sessionId, String json) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.setProperty(json);
    }

    public static void videoDestroy(int sessionId, int videoId) {
        VideoManager mgr = getOrCreateVideoManager(sessionId);
        if (mgr == null) return;
        mgr.destroyVideo(videoId);
    }

    public static void destroyVideoManager(int sessionId) {
        VideoManager mgr = sVideoManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    // ==================== Bulk Destroy ====================

    public static void destroyAll(int sessionId) {
        destroyRecorderManager(sessionId);
        destroyCameraManagers(sessionId);
        destroyImageApiManager(sessionId);
        destroyVideoManager(sessionId);
    }
}
