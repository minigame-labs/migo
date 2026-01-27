package com.migo.host.internal.jni;

import com.migo.host.InitOption;

import java.nio.ByteBuffer;

/**
 * JNI bridge class containing native method declarations.
 * <p>
 * These methods are implemented in Rust and registered via JNI_OnLoad.
 * Do not call these methods directly - use {@link HostNative} instead.
 */
public final class HostJNI {

    static {
        System.loadLibrary("migo");
    }

    private HostJNI() {
        // Prevent instantiation
    }

    /**
     * Get the native engine version string.
     *
     * @return Version string (e.g., "0.1.0")
     */
    public static native String version();

    /**
     * Initialize a new host instance with the given surface and options.
     *
     * @param surface Android Surface object for rendering
     * @param options Initialization options
     * @return Host ID (>= 0) on success, -1 on failure
     */
    public static native int init(Object surface, InitOption options);

    /**
     * Shut down a host instance and release all resources.
     *
     * @param hostId The host ID returned by init()
     */
    public static native void shutdown(int hostId);

    /**
     * Notify the host that a surface has been updated/recreated.
     *
     * @param hostId  The host ID
     * @param surface New Android Surface object
     */
    public static native void updateSurface(int hostId, Object surface);

    /**
     * Notify the host that it should show (resume rendering).
     *
     * @param hostId The host ID
     */
    public static native void onShow(int hostId);

    /**
     * Notify the host that it should hide (pause rendering).
     *
     * @param hostId The host ID
     */
    public static native void onHide(int hostId);

    /**
     * Send touch events to the host.
     *
     * @param hostId The host ID
     * @param action Touch action (0=DOWN, 1=UP, 2=MOVE, 3=CANCEL, 5=POINTER_DOWN, 6=POINTER_UP)
     * @param time   Event timestamp in milliseconds
     * @param count  Number of touch points
     * @param buffer DirectByteBuffer containing packed TouchPoint data
     */
    public static native void onTouchEvent(int hostId, int action, long time, int count, ByteBuffer buffer);

    /**
     * Load and run a game module.
     *
     * @param hostId  The host ID
     * @param codeDir Directory containing the game code
     * @param entry   Entry point file name (e.g., "game.js")
     * @return 0 on success, -1 on failure
     */
    public static native int modMain(int hostId, String codeDir, String entry);

    /**
     * Callback when system Bluetooth setting result is received.
     *
     * @param hostId  The host ID
     * @param enabled 1 if Bluetooth was enabled, 0 otherwise
     */
    public static native void onOpenSystemBluetoothSetting(int hostId, int enabled);
}
