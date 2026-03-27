package com.migo.runtime.internal.platform;

import android.content.ContentResolver;
import android.content.Context;
import android.database.ContentObserver;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;

import com.migo.runtime.internal.NativeMethods;

/**
 * Observes screenshot events by monitoring MediaStore changes.
 * <p>
 * When the user takes a screenshot, Android writes the image to MediaStore.
 * This observer detects that by querying the actual file path/name of newly
 * added images and checking for screenshot-related keywords.
 * <p>
 * On Android 10+ (API 29+), no special permission is needed to query
 * MediaStore metadata (DISPLAY_NAME, RELATIVE_PATH). On older versions,
 * READ_EXTERNAL_STORAGE may be required but those devices are rare now.
 *
 * @hide
 */
public final class ScreenCaptureObserver {

    private static final String TAG = "ScreenCaptureObserver";
    private static final long DEBOUNCE_MS = 1000;

    private static final String[] PROJECTION;

    static {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            PROJECTION = new String[]{
                    MediaStore.Images.Media.DISPLAY_NAME,
                    MediaStore.Images.Media.RELATIVE_PATH,
                    MediaStore.Images.Media.DATE_ADDED,
            };
        } else {
            PROJECTION = new String[]{
                    MediaStore.Images.Media.DATA,
                    MediaStore.Images.Media.DATE_ADDED,
            };
        }
    }

    private final int sessionId;
    private final ContentResolver contentResolver;
    private final Handler handler;

    private ContentObserver observer;
    private long lastNotifyTime = 0;

    public ScreenCaptureObserver(int sessionId, Context context) {
        this.sessionId = sessionId;
        this.contentResolver = context.getContentResolver();
        this.handler = new Handler(Looper.getMainLooper());
    }

    /**
     * Start observing screenshot events.
     */
    public void start() {
        stop();

        observer = new ContentObserver(handler) {
            @Override
            public void onChange(boolean selfChange, Uri uri) {
                if (uri == null) return;
                handleChange(uri);
            }
        };

        try {
            contentResolver.registerContentObserver(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    true,
                    observer
            );
        } catch (Exception e) {
            android.util.Log.w(TAG, "Failed to register observer", e);
        }
    }

    /**
     * Stop observing screenshot events.
     */
    public void stop() {
        if (observer != null) {
            try {
                contentResolver.unregisterContentObserver(observer);
            } catch (Exception ignored) {
            }
            observer = null;
        }
    }

    /**
     * Clean up resources.
     */
    public void destroy() {
        stop();
    }

    private void handleChange(Uri uri) {
        // Debounce: screenshots often trigger multiple onChange calls
        long now = System.currentTimeMillis();
        if (now - lastNotifyTime < DEBOUNCE_MS) {
            return;
        }

        // Quick check: if the URI string itself contains screenshot keywords, skip the query
        String uriStr = uri.toString().toLowerCase();
        if (matchesScreenshot(uriStr)) {
            notifyCapture(now);
            return;
        }

        // Query MediaStore for the actual file info
        if (isScreenshotUri(uri)) {
            notifyCapture(now);
        }
    }

    private void notifyCapture(long now) {
        lastNotifyTime = now;
        NativeMethods.onUserCaptureScreen(sessionId);
    }

    private boolean isScreenshotUri(Uri uri) {
        Cursor cursor = null;
        try {
            // Query the most recent image. On API 26+ use Bundle-based query
            // arguments for proper LIMIT support; on older APIs use the LIMIT
            // trick appended to the sort order string.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                Bundle queryArgs = new Bundle();
                queryArgs.putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER,
                        MediaStore.Images.Media.DATE_ADDED + " DESC");
                queryArgs.putInt(ContentResolver.QUERY_ARG_LIMIT, 1);
                cursor = contentResolver.query(
                        MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                        PROJECTION, queryArgs, null);
            } else {
                String sortOrder = MediaStore.Images.Media.DATE_ADDED + " DESC LIMIT 1";
                cursor = contentResolver.query(
                        MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                        PROJECTION, null, null, sortOrder);
            }
            if (cursor != null && cursor.moveToFirst()) {
                // Verify the image was added recently (within the last few seconds)
                int dateIdx = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_ADDED);
                long dateAdded = cursor.getLong(dateIdx);
                long nowSeconds = System.currentTimeMillis() / 1000;
                if (Math.abs(nowSeconds - dateAdded) > 5) {
                    return false;
                }

                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    String displayName = cursor.getString(
                            cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME));
                    String relativePath = cursor.getString(
                            cursor.getColumnIndexOrThrow(MediaStore.Images.Media.RELATIVE_PATH));

                    // Check file name and path for screenshot keywords
                    if (displayName != null && matchesScreenshot(displayName.toLowerCase())) {
                        return true;
                    }
                    if (relativePath != null && matchesScreenshot(relativePath.toLowerCase())) {
                        return true;
                    }
                } else {
                    // API < 29: use DATA column (absolute file path)
                    @SuppressWarnings("deprecation")
                    String data = cursor.getString(
                            cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DATA));
                    if (data != null && matchesScreenshot(data.toLowerCase())) {
                        return true;
                    }
                }
            }
        } catch (Exception e) {
            // SecurityException, IllegalArgumentException, etc.
            // Silently ignore - we can't determine if it's a screenshot
        } finally {
            if (cursor != null) {
                cursor.close();
            }
        }
        return false;
    }

    private static boolean matchesScreenshot(String s) {
        return s.contains("screenshot")
                || s.contains("screen_shot")
                || s.contains("screen-shot")
                || s.contains("screenshots")
                || s.contains("截屏")
                || s.contains("截图");
    }
}
