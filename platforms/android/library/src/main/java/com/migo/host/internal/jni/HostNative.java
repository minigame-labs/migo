package com.migo.host.internal.jni;

import android.view.Surface;

import com.migo.host.InitOption;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * High-level wrapper around native JNI methods.
 * <p>
 * Provides a safer API with proper parameter validation and error handling.
 * This is the recommended way to interact with the native engine.
 */
public final class HostNative {

    /**
     * Size of a single TouchPoint structure in bytes.
     * Layout: id(4) + x(4) + y(4) + force(4) = 16 bytes
     */
    private static final int TOUCH_POINT_SIZE = 16;

    /**
     * Maximum number of simultaneous touch points supported.
     */
    private static final int MAX_TOUCH_POINTS = 10;

    /**
     * Thread-local ByteBuffer for touch events to avoid allocation per event.
     */
    private static final ThreadLocal<ByteBuffer> sTouchBuffer = new ThreadLocal<ByteBuffer>() {
        @Override
        protected ByteBuffer initialValue() {
            ByteBuffer buffer = ByteBuffer.allocateDirect(TOUCH_POINT_SIZE * MAX_TOUCH_POINTS);
            buffer.order(ByteOrder.nativeOrder());
            return buffer;
        }
    };

    private HostNative() {
        // Prevent instantiation
    }

    /**
     * Get the native engine version.
     *
     * @return Version string, or "unknown" if native library failed to load
     */
    public static String getVersion() {
        try {
            String version = HostJNI.version();
            return version != null ? version : "unknown";
        } catch (UnsatisfiedLinkError e) {
            return "unknown";
        }
    }

    /**
     * Initialize a new host instance.
     *
     * @param surface Android Surface for rendering
     * @param options Initialization options
     * @return Host ID (>= 0) on success, -1 on failure
     * @throws IllegalArgumentException if surface or options is null
     */
    public static int init(Surface surface, InitOption options) {
        if (surface == null) {
            throw new IllegalArgumentException("Surface cannot be null");
        }
        if (options == null) {
            throw new IllegalArgumentException("InitOption cannot be null");
        }
        return HostJNI.init(surface, options);
    }

    /**
     * Shut down a host instance.
     *
     * @param hostId The host ID to shut down
     */
    public static void shutdown(int hostId) {
        if (hostId >= 0) {
            HostJNI.shutdown(hostId);
        }
    }

    /**
     * Update the rendering surface.
     *
     * @param hostId  The host ID
     * @param surface New Surface object
     */
    public static void updateSurface(int hostId, Surface surface) {
        if (hostId >= 0 && surface != null) {
            HostJNI.updateSurface(hostId, surface);
        }
    }

    /**
     * Notify host that it should resume.
     *
     * @param hostId The host ID
     */
    public static void onShow(int hostId) {
        if (hostId >= 0) {
            HostJNI.onShow(hostId);
        }
    }

    /**
     * Notify host that it should pause.
     *
     * @param hostId The host ID
     */
    public static void onHide(int hostId) {
        if (hostId >= 0) {
            HostJNI.onHide(hostId);
        }
    }

    /**
     * Send touch events to the host.
     *
     * @param hostId The host ID
     * @param action Touch action code
     * @param time   Event timestamp in milliseconds
     * @param ids    Array of pointer IDs
     * @param xs     Array of X coordinates
     * @param ys     Array of Y coordinates
     * @param forces Array of pressure values (0.0-1.0)
     */
    public static void onTouch(int hostId, int action, long time,
                               int[] ids, float[] xs, float[] ys, float[] forces) {
        if (hostId < 0 || ids == null || ids.length == 0) {
            return;
        }

        int count = Math.min(ids.length, MAX_TOUCH_POINTS);
        ByteBuffer buffer = sTouchBuffer.get();
        buffer.clear();

        for (int i = 0; i < count; i++) {
            buffer.putInt(ids[i]);
            buffer.putFloat(xs[i]);
            buffer.putFloat(ys[i]);
            buffer.putFloat(forces != null && i < forces.length ? forces[i] : 1.0f);
        }

        buffer.flip();
        HostJNI.onTouchEvent(hostId, action, time, count, buffer);
    }

    /**
     * Load and run a game module.
     *
     * @param hostId  The host ID
     * @param codeDir Directory containing game code
     * @param entry   Entry point file (e.g., "game.js")
     * @return 0 on success, -1 on failure
     */
    public static int modMain(int hostId, String codeDir, String entry) {
        if (hostId < 0 || codeDir == null || entry == null) {
            return -1;
        }
        return HostJNI.modMain(hostId, codeDir, entry);
    }

    /**
     * Callback for Bluetooth setting result.
     *
     * @param hostId  The host ID
     * @param enabled Whether Bluetooth is now enabled
     */
    public static void onBluetoothSettingResult(int hostId, boolean enabled) {
        if (hostId >= 0) {
            HostJNI.onOpenSystemBluetoothSetting(hostId, enabled ? 1 : 0);
        }
    }
}
