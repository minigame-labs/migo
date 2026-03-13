package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.util.Log;
import android.webkit.MimeTypeMap;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

/**
 * Manages image-related APIs for a session.
 * Sync-style APIs (save, preview, compress) throw on failure.
 * Async-style APIs (chooseImage, chooseMessageFile) call back via
 * {@link NativeMethods#onChooseImageResult} / {@link NativeMethods#onChooseMessageFileResult}.
 *
 * @hide
 */
public class ImageApiManager {

    private static final String TAG = "ImageApiManager";

    private static final int REQUEST_CHOOSE_IMAGE = 9001;
    private static final int REQUEST_CAPTURE_IMAGE = 9002;
    private static final int REQUEST_CHOOSE_FILE = 9003;

    private final int sessionId;
    private final Activity activity;
    private final Handler mainHandler;

    // Pending chooseImage state
    private int pendingChooseImageCount = 9;
    private boolean pendingChooseImageCompress = false;
    private Uri pendingCameraTempUri = null;

    public ImageApiManager(int sessionId, Activity activity) {
        this.sessionId = sessionId;
        this.activity = activity;
        this.mainHandler = new Handler(Looper.getMainLooper());
    }

    // ==================== saveImageToPhotosAlbum ====================

    /**
     * Save image to system photo album via MediaStore.
     * Options JSON: { "filePath": "..." }
     */
    public void saveToPhotosAlbum(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String filePath = opts.getString("filePath");

            File srcFile = new File(filePath);
            if (!srcFile.exists()) {
                throw new RuntimeException("saveImageToPhotosAlbum:fail file not found");
            }

            // Decode to verify it's a valid image and get format
            BitmapFactory.Options bmOpts = new BitmapFactory.Options();
            bmOpts.inJustDecodeBounds = true;
            BitmapFactory.decodeFile(filePath, bmOpts);
            String mimeType = bmOpts.outMimeType;
            if (mimeType == null) {
                mimeType = "image/jpeg";
            }

            String displayName = srcFile.getName();
            String extension = "";
            int dotIdx = displayName.lastIndexOf('.');
            if (dotIdx > 0) {
                extension = displayName.substring(dotIdx);
            }
            if (extension.isEmpty()) {
                extension = mimeType.contains("png") ? ".png" : ".jpg";
            }

            ContentResolver resolver = activity.getContentResolver();
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME, "IMG_" + System.currentTimeMillis() + extension);
            values.put(MediaStore.Images.Media.MIME_TYPE, mimeType);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                values.put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES);
                values.put(MediaStore.Images.Media.IS_PENDING, 1);
            }

            Uri uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
            if (uri == null) {
                throw new RuntimeException("saveImageToPhotosAlbum:fail insert failed");
            }

            try (InputStream in = new java.io.FileInputStream(srcFile);
                 OutputStream out = resolver.openOutputStream(uri)) {
                if (out == null) {
                    throw new RuntimeException("saveImageToPhotosAlbum:fail cannot open output");
                }
                byte[] buf = new byte[8192];
                int len;
                while ((len = in.read(buf)) > 0) {
                    out.write(buf, 0, len);
                }
            }

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                values.clear();
                values.put(MediaStore.Images.Media.IS_PENDING, 0);
                resolver.update(uri, values, null, null);
            }

            Log.d(TAG, "saveToPhotosAlbum: saved to " + uri);
        } catch (RuntimeException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("saveImageToPhotosAlbum:fail " + e.getMessage());
        }
    }

    // ==================== previewMedia ====================

    /**
     * Preview media (images and videos) in fullscreen using system viewer.
     * Options JSON: { "sources": [{"url":"...","type":"image|video","poster":"..."}], "current": 0 }
     */
    public void previewMedia(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            JSONArray sources = opts.getJSONArray("sources");
            int current = opts.optInt("current", 0);

            if (sources.length() == 0) {
                throw new RuntimeException("previewMedia:fail sources is empty");
            }
            if (current < 0 || current >= sources.length()) {
                current = 0;
            }

            JSONObject item = sources.getJSONObject(current);
            String url = item.getString("url");
            String type = item.optString("type", "image");

            Uri uri = resolveUri(url);
            String mime = "video".equals(type) ? "video/*" : "image/*";

            Intent intent = new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri, mime);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            activity.startActivity(intent);
        } catch (RuntimeException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("previewMedia:fail " + e.getMessage());
        }
    }

    // ==================== previewImage ====================

    /**
     * Preview images in fullscreen using system viewer.
     * Options JSON: { "urls": [...], "current": "..." }
     */
    public void previewImage(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            JSONArray urls = opts.getJSONArray("urls");
            String current = opts.optString("current", "");

            if (urls.length() == 0) {
                throw new RuntimeException("previewImage:fail urls is empty");
            }

            // Find the current URL or default to first
            String targetUrl = current;
            if (targetUrl.isEmpty()) {
                targetUrl = urls.getString(0);
            }

            Uri uri = resolveUri(targetUrl);

            Intent intent = new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri, "image/*");
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            activity.startActivity(intent);
        } catch (RuntimeException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("previewImage:fail " + e.getMessage());
        }
    }

    // ==================== compressImage (async) ====================

    /**
     * Compress an image asynchronously.
     * Options JSON: { "src": "...", "quality": 80, "compressedWidth": 0, "compressedHeight": 0 }
     * Result delivered via NativeMethods.onCompressImageResult with { "tempFilePath": "..." }
     */
    public void compressAsync(final int sessionId, final String optionsJson) {
        new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    String resultJson = compressSync(optionsJson);
                    NativeMethods.onCompressImageResult(sessionId, resultJson);
                } catch (Exception e) {
                    String msg = e.getMessage();
                    if (msg == null) msg = "unknown error";
                    NativeMethods.onCompressImageResult(sessionId,
                            "{\"error\":\"" + msg.replace("\"", "\\\"") + "\"}");
                }
            }
        }, "migo-compress-image").start();
    }

    private String compressSync(String optionsJson) throws Exception {
        JSONObject opts = new JSONObject(optionsJson);
        String src = opts.getString("src");
        int quality = opts.optInt("quality", 80);
        int targetWidth = opts.optInt("compressedWidth", 0);
        int targetHeight = opts.optInt("compressedHeight", 0);

        quality = Math.max(0, Math.min(100, quality));

        File srcFile = new File(src);
        if (!srcFile.exists()) {
            throw new RuntimeException("compressImage:fail file not found");
        }

        // First pass: get original dimensions
        BitmapFactory.Options bmOpts = new BitmapFactory.Options();
        bmOpts.inJustDecodeBounds = true;
        BitmapFactory.decodeFile(src, bmOpts);
        int origWidth = bmOpts.outWidth;
        int origHeight = bmOpts.outHeight;

        if (origWidth <= 0 || origHeight <= 0) {
            throw new RuntimeException("compressImage:fail invalid image");
        }

        // Calculate target dimensions
        int finalWidth = origWidth;
        int finalHeight = origHeight;
        if (targetWidth > 0 && targetHeight > 0) {
            finalWidth = targetWidth;
            finalHeight = targetHeight;
        } else if (targetWidth > 0) {
            float ratio = (float) targetWidth / origWidth;
            finalWidth = targetWidth;
            finalHeight = Math.round(origHeight * ratio);
        } else if (targetHeight > 0) {
            float ratio = (float) targetHeight / origHeight;
            finalHeight = targetHeight;
            finalWidth = Math.round(origWidth * ratio);
        }

        // Calculate inSampleSize for efficient decoding
        bmOpts.inJustDecodeBounds = false;
        bmOpts.inSampleSize = calculateInSampleSize(origWidth, origHeight, finalWidth, finalHeight);

        Bitmap bitmap = BitmapFactory.decodeFile(src, bmOpts);
        if (bitmap == null) {
            throw new RuntimeException("compressImage:fail decode failed");
        }

        // Scale to exact target if needed
        if (bitmap.getWidth() != finalWidth || bitmap.getHeight() != finalHeight) {
            Bitmap scaled = Bitmap.createScaledBitmap(bitmap, finalWidth, finalHeight, true);
            if (scaled != bitmap) {
                bitmap.recycle();
            }
            bitmap = scaled;
        }

        // Determine output format
        String mimeType = bmOpts.outMimeType;
        Bitmap.CompressFormat format = Bitmap.CompressFormat.JPEG;
        String ext = ".jpg";
        if (mimeType != null && mimeType.contains("png")) {
            format = Bitmap.CompressFormat.PNG;
            ext = ".png";
        }

        // Write compressed to temp file
        File tempFile = createTempFile("compress", ext);
        try (FileOutputStream fos = new FileOutputStream(tempFile)) {
            boolean ok = bitmap.compress(format, quality, fos);
            if (!ok) {
                throw new RuntimeException("compressImage:fail compress failed");
            }
        } finally {
            bitmap.recycle();
        }

        JSONObject result = new JSONObject();
        result.put("tempFilePath", tempFile.getAbsolutePath());
        return result.toString();
    }

    // ==================== chooseMessageFile (async) ====================

    /**
     * Choose files from system file picker (async, results via callback).
     * Options JSON: { "count": 10, "type": "all", "extension": [] }
     */
    public void chooseMessageFile(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            int count = opts.optInt("count", 1);
            String type = opts.optString("type", "all");
            JSONArray extensionArr = opts.optJSONArray("extension");

            String mimeType;
            switch (type) {
                case "image":
                    mimeType = "image/*";
                    break;
                case "video":
                    mimeType = "video/*";
                    break;
                case "file":
                    mimeType = "*/*";
                    break;
                default: // "all"
                    mimeType = "*/*";
                    break;
            }

            mainHandler.post(() -> {
                try {
                    Intent intent = new Intent(Intent.ACTION_GET_CONTENT);
                    intent.setType(mimeType);
                    intent.addCategory(Intent.CATEGORY_OPENABLE);
                    if (count > 1) {
                        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
                    }
                    activity.startActivityForResult(
                            Intent.createChooser(intent, "Choose File"),
                            REQUEST_CHOOSE_FILE);
                } catch (Exception e) {
                    Log.e(TAG, "chooseMessageFile: failed to launch picker", e);
                    sendChooseMessageFileError("chooseMessageFile:fail " + e.getMessage());
                }
            });
        } catch (Exception e) {
            throw new RuntimeException("chooseMessageFile:fail " + e.getMessage());
        }
    }

    // ==================== chooseImage (async) ====================

    /**
     * Choose images from album or camera (async, results via callback).
     * Options JSON: { "count": 9, "sizeType": [...], "sourceType": [...] }
     */
    public void chooseImage(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            int count = opts.optInt("count", 9);
            JSONArray sourceType = opts.optJSONArray("sourceType");

            boolean hasAlbum = true;
            boolean hasCamera = true;
            if (sourceType != null) {
                hasAlbum = jsonArrayContains(sourceType, "album");
                hasCamera = jsonArrayContains(sourceType, "camera");
            }

            JSONArray sizeType = opts.optJSONArray("sizeType");
            pendingChooseImageCount = count;
            pendingChooseImageCompress = sizeType != null
                    && jsonArrayContains(sizeType, "compressed")
                    && !jsonArrayContains(sizeType, "original");

            final boolean useAlbum = hasAlbum;
            final boolean useCamera = hasCamera;

            mainHandler.post(() -> {
                try {
                    if (useAlbum && !useCamera) {
                        // Album only
                        launchImagePicker(count);
                    } else if (useCamera && !useAlbum) {
                        // Camera only
                        launchCameraCapture();
                    } else {
                        // Both: use a chooser with camera + gallery
                        launchImagePicker(count);
                    }
                } catch (Exception e) {
                    Log.e(TAG, "chooseImage: failed to launch", e);
                    sendChooseImageError("chooseImage:fail " + e.getMessage());
                }
            });
        } catch (Exception e) {
            throw new RuntimeException("chooseImage:fail " + e.getMessage());
        }
    }

    // ==================== ActivityResult handling ====================

    /**
     * Must be called from the host Activity's onActivityResult.
     */
    public void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (resultCode != Activity.RESULT_OK) {
            if (requestCode == REQUEST_CHOOSE_IMAGE || requestCode == REQUEST_CAPTURE_IMAGE) {
                sendChooseImageError("chooseImage:fail cancel");
            } else if (requestCode == REQUEST_CHOOSE_FILE) {
                sendChooseMessageFileError("chooseMessageFile:fail cancel");
            }
            return;
        }

        switch (requestCode) {
            case REQUEST_CHOOSE_IMAGE:
                handleChooseImageResult(data);
                break;
            case REQUEST_CAPTURE_IMAGE:
                handleCaptureImageResult();
                break;
            case REQUEST_CHOOSE_FILE:
                handleChooseFileResult(data);
                break;
        }
    }

    /**
     * Release resources when session is destroyed.
     */
    public void destroy() {
        pendingCameraTempUri = null;
    }

    // ========================================================================
    // Internal: chooseImage result handling
    // ========================================================================

    private void handleChooseImageResult(Intent data) {
        try {
            List<String> paths = new ArrayList<>();
            List<Long> sizes = new ArrayList<>();

            if (data.getClipData() != null) {
                int clipCount = Math.min(data.getClipData().getItemCount(), pendingChooseImageCount);
                for (int i = 0; i < clipCount; i++) {
                    Uri uri = data.getClipData().getItemAt(i).getUri();
                    String path = copyUriToTemp(uri, "chooseimg", ".jpg");
                    if (path != null) {
                        paths.add(path);
                        sizes.add(new File(path).length());
                    }
                }
            } else if (data.getData() != null) {
                String path = copyUriToTemp(data.getData(), "chooseimg", ".jpg");
                if (path != null) {
                    paths.add(path);
                    sizes.add(new File(path).length());
                }
            }

            if (paths.isEmpty()) {
                sendChooseImageError("chooseImage:fail no image selected");
                return;
            }

            JSONObject result = new JSONObject();
            JSONArray tempFilePaths = new JSONArray();
            JSONArray tempFiles = new JSONArray();
            for (int i = 0; i < paths.size(); i++) {
                tempFilePaths.put(paths.get(i));
                JSONObject f = new JSONObject();
                f.put("path", paths.get(i));
                f.put("size", sizes.get(i));
                tempFiles.put(f);
            }
            result.put("tempFilePaths", tempFilePaths);
            result.put("tempFiles", tempFiles);

            NativeMethods.onChooseImageResult(sessionId, result.toString());
        } catch (Exception e) {
            Log.e(TAG, "handleChooseImageResult error", e);
            sendChooseImageError("chooseImage:fail " + e.getMessage());
        }
    }

    private void handleCaptureImageResult() {
        try {
            if (pendingCameraTempUri == null) {
                sendChooseImageError("chooseImage:fail no capture uri");
                return;
            }

            String path = copyUriToTemp(pendingCameraTempUri, "capture", ".jpg");
            pendingCameraTempUri = null;

            if (path == null) {
                sendChooseImageError("chooseImage:fail copy failed");
                return;
            }

            JSONObject result = new JSONObject();
            JSONArray tempFilePaths = new JSONArray();
            JSONArray tempFiles = new JSONArray();
            tempFilePaths.put(path);
            JSONObject f = new JSONObject();
            f.put("path", path);
            f.put("size", new File(path).length());
            tempFiles.put(f);
            result.put("tempFilePaths", tempFilePaths);
            result.put("tempFiles", tempFiles);

            NativeMethods.onChooseImageResult(sessionId, result.toString());
        } catch (Exception e) {
            Log.e(TAG, "handleCaptureImageResult error", e);
            sendChooseImageError("chooseImage:fail " + e.getMessage());
        }
    }

    // ========================================================================
    // Internal: chooseMessageFile result handling
    // ========================================================================

    private void handleChooseFileResult(Intent data) {
        try {
            List<JSONObject> files = new ArrayList<>();

            if (data.getClipData() != null) {
                int clipCount = data.getClipData().getItemCount();
                for (int i = 0; i < clipCount; i++) {
                    Uri uri = data.getClipData().getItemAt(i).getUri();
                    JSONObject info = resolveFileInfo(uri);
                    if (info != null) {
                        files.add(info);
                    }
                }
            } else if (data.getData() != null) {
                JSONObject info = resolveFileInfo(data.getData());
                if (info != null) {
                    files.add(info);
                }
            }

            if (files.isEmpty()) {
                sendChooseMessageFileError("chooseMessageFile:fail no file selected");
                return;
            }

            JSONObject result = new JSONObject();
            JSONArray tempFiles = new JSONArray();
            for (JSONObject f : files) {
                tempFiles.put(f);
            }
            result.put("tempFiles", tempFiles);

            NativeMethods.onChooseMessageFileResult(sessionId, result.toString());
        } catch (Exception e) {
            Log.e(TAG, "handleChooseFileResult error", e);
            sendChooseMessageFileError("chooseMessageFile:fail " + e.getMessage());
        }
    }

    /**
     * Resolve file info from a content URI: copy to temp, get name/size/type.
     */
    private JSONObject resolveFileInfo(Uri uri) {
        try {
            ContentResolver resolver = activity.getContentResolver();
            String name = "unknown";
            long size = 0;

            try (Cursor cursor = resolver.query(uri, null, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    int sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE);
                    if (nameIdx >= 0) name = cursor.getString(nameIdx);
                    if (sizeIdx >= 0) size = cursor.getLong(sizeIdx);
                }
            }

            // Determine type from MIME
            String mime = resolver.getType(uri);
            String fileType = "file";
            if (mime != null) {
                if (mime.startsWith("image/")) fileType = "image";
                else if (mime.startsWith("video/")) fileType = "video";
            }

            // Determine extension
            String ext = "";
            int dotIdx = name.lastIndexOf('.');
            if (dotIdx > 0) {
                ext = name.substring(dotIdx);
            } else if (mime != null) {
                String guessed = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime);
                if (guessed != null) ext = "." + guessed;
            }

            String tempPath = copyUriToTemp(uri, "msgfile", ext.isEmpty() ? ".tmp" : ext);
            if (tempPath == null) return null;

            if (size == 0) {
                size = new File(tempPath).length();
            }

            JSONObject info = new JSONObject();
            info.put("path", tempPath);
            info.put("size", size);
            info.put("name", name);
            info.put("type", fileType);
            info.put("time", System.currentTimeMillis() / 1000);
            return info;
        } catch (Exception e) {
            Log.e(TAG, "resolveFileInfo error", e);
            return null;
        }
    }

    // ========================================================================
    // Intent launchers
    // ========================================================================

    private void launchImagePicker(int count) {
        Intent intent = new Intent(Intent.ACTION_GET_CONTENT);
        intent.setType("image/*");
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        if (count > 1) {
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        }
        activity.startActivityForResult(
                Intent.createChooser(intent, "Choose Image"),
                REQUEST_CHOOSE_IMAGE);
    }

    private void launchCameraCapture() {
        Intent intent = new Intent(MediaStore.ACTION_IMAGE_CAPTURE);
        if (intent.resolveActivity(activity.getPackageManager()) == null) {
            sendChooseImageError("chooseImage:fail no camera app");
            return;
        }

        try {
            // Use MediaStore to create a URI the camera app can write to
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME, "capture_" + System.currentTimeMillis() + ".jpg");
            values.put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg");
            pendingCameraTempUri = activity.getContentResolver().insert(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
            if (pendingCameraTempUri == null) {
                sendChooseImageError("chooseImage:fail cannot create capture uri");
                return;
            }
            intent.putExtra(MediaStore.EXTRA_OUTPUT, pendingCameraTempUri);
            intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            activity.startActivityForResult(intent, REQUEST_CAPTURE_IMAGE);
        } catch (Exception e) {
            Log.e(TAG, "launchCameraCapture error", e);
            sendChooseImageError("chooseImage:fail " + e.getMessage());
        }
    }

    // ========================================================================
    // Utility
    // ========================================================================

    private File createTempFile(String prefix, String extension) throws IOException {
        File cacheDir = new File(activity.getCacheDir(), "image_api");
        if (!cacheDir.exists()) {
            cacheDir.mkdirs();
        }
        return new File(cacheDir, prefix + "_" + System.currentTimeMillis() + extension);
    }

    /**
     * Copy content from a Uri to a temp file and return the temp path.
     */
    private String copyUriToTemp(Uri uri, String prefix, String ext) {
        try {
            File tempFile = createTempFile(prefix, ext);
            try (InputStream in = activity.getContentResolver().openInputStream(uri);
                 FileOutputStream out = new FileOutputStream(tempFile)) {
                if (in == null) return null;
                byte[] buf = new byte[8192];
                int len;
                while ((len = in.read(buf)) > 0) {
                    out.write(buf, 0, len);
                }
            }
            return tempFile.getAbsolutePath();
        } catch (Exception e) {
            Log.e(TAG, "copyUriToTemp error", e);
            return null;
        }
    }

    private Uri resolveUri(String url) {
        if (url.startsWith("content://") || url.startsWith("file://")) {
            return Uri.parse(url);
        }
        // Local file path
        File f = new File(url);
        if (f.exists()) {
            return Uri.fromFile(f);
        }
        // Treat as a URL (http/https)
        return Uri.parse(url);
    }

    private int calculateInSampleSize(int srcW, int srcH, int reqW, int reqH) {
        int inSampleSize = 1;
        if (srcH > reqH || srcW > reqW) {
            int halfH = srcH / 2;
            int halfW = srcW / 2;
            while ((halfH / inSampleSize) >= reqH && (halfW / inSampleSize) >= reqW) {
                inSampleSize *= 2;
            }
        }
        return inSampleSize;
    }

    private void sendChooseImageError(String errMsg) {
        try {
            JSONObject err = new JSONObject();
            err.put("error", errMsg);
            NativeMethods.onChooseImageResult(sessionId, err.toString());
        } catch (Exception e) {
            Log.e(TAG, "sendChooseImageError failed", e);
        }
    }

    private void sendChooseMessageFileError(String errMsg) {
        try {
            JSONObject err = new JSONObject();
            err.put("error", errMsg);
            NativeMethods.onChooseMessageFileResult(sessionId, err.toString());
        } catch (Exception e) {
            Log.e(TAG, "sendChooseMessageFileError failed", e);
        }
    }

    private static boolean jsonArrayContains(JSONArray arr, String value) {
        if (arr == null) return false;
        for (int i = 0; i < arr.length(); i++) {
            if (value.equals(arr.optString(i))) return true;
        }
        return false;
    }
}
