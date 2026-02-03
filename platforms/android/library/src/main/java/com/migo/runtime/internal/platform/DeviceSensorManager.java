package com.migo.runtime.internal.platform;

import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.view.Display;
import android.view.Surface;
import android.view.WindowManager;

import com.migo.runtime.internal.NativeMethods;

/**
 * Manages device sensor listeners for motion, gyroscope, and orientation.
 * <p>
 * Uses Android SensorManager to listen for sensor events and dispatches
 * data to the native layer via {@link NativeMethods}.
 * <p>
 * Interval mapping from Mini Game spec:
 * <ul>
 *   <li>"game"   = ~20ms  (SENSOR_DELAY_GAME)</li>
 *   <li>"ui"     = ~60ms  (SENSOR_DELAY_UI)</li>
 *   <li>"normal" = ~200ms (SENSOR_DELAY_NORMAL)</li>
 * </ul>
 * <p>
 * Compatible with Android API 21+.
 *
 * @hide
 */
public final class DeviceSensorManager {

    private final int sessionId;
    private final SensorManager sensorManager;
    private final WindowManager windowManager;

    private SensorEventListener motionListener;
    private SensorEventListener gyroscopeListener;

    public DeviceSensorManager(int sessionId, Context context) {
        this.sessionId = sessionId;
        this.sensorManager = (SensorManager) context.getSystemService(Context.SENSOR_SERVICE);
        this.windowManager = (WindowManager) context.getSystemService(Context.WINDOW_SERVICE);
    }

    // ==================== Device Motion ====================

    /**
     * Start listening for device motion (rotation vector) events.
     * <p>
     * Uses TYPE_ROTATION_VECTOR sensor and converts quaternion output to
     * Euler angles (alpha, beta, gamma) following the W3C DeviceOrientation spec.
     *
     * @param interval "game", "ui", or "normal"
     */
    public void startDeviceMotionListening(String interval) {
        if (sensorManager == null) return;
        stopDeviceMotionListening();

        Sensor sensor = sensorManager.getDefaultSensor(Sensor.TYPE_ROTATION_VECTOR);
        if (sensor == null) {
            // Fallback to game rotation vector (no magnetic field, less accurate but more available)
            sensor = sensorManager.getDefaultSensor(Sensor.TYPE_GAME_ROTATION_VECTOR);
        }
        if (sensor == null) return;

        final int delay = parseInterval(interval);

        motionListener = new SensorEventListener() {
            private final float[] rotationMatrix = new float[9];
            private final float[] orientation = new float[3];

            @Override
            public void onSensorChanged(SensorEvent event) {
                SensorManager.getRotationMatrixFromVector(rotationMatrix, event.values);

                // Remap coordinate system based on display rotation for correct
                // alpha/beta/gamma regardless of screen orientation
                float[] remapped = remapForDisplay(rotationMatrix);

                SensorManager.getOrientation(remapped, orientation);

                // Convert from radians to degrees
                // orientation[0] = azimuth (rotation around Z), -PI..PI -> alpha 0..360
                // orientation[1] = pitch (rotation around X), -PI..PI -> beta -180..180
                // orientation[2] = roll (rotation around Y), -PI/2..PI/2 -> gamma -90..90
                double alpha = Math.toDegrees(orientation[0]);
                if (alpha < 0) alpha += 360.0;
                double beta = Math.toDegrees(orientation[1]);
                double gamma = Math.toDegrees(orientation[2]);

                NativeMethods.onDeviceMotionChange(sessionId, alpha, beta, gamma);
            }

            @Override
            public void onAccuracyChanged(Sensor sensor, int accuracy) {
            }
        };

        sensorManager.registerListener(motionListener, sensor, delay);
    }

    /**
     * Stop listening for device motion events.
     */
    public void stopDeviceMotionListening() {
        if (sensorManager != null && motionListener != null) {
            sensorManager.unregisterListener(motionListener);
            motionListener = null;
        }
    }

    // ==================== Gyroscope ====================

    /**
     * Start listening for gyroscope events.
     * <p>
     * Reports angular velocity in rad/s around x, y, z axes.
     *
     * @param interval "game", "ui", or "normal"
     */
    public void startGyroscope(String interval) {
        if (sensorManager == null) return;
        stopGyroscope();

        Sensor sensor = sensorManager.getDefaultSensor(Sensor.TYPE_GYROSCOPE);
        if (sensor == null) return;

        final int delay = parseInterval(interval);

        gyroscopeListener = new SensorEventListener() {
            @Override
            public void onSensorChanged(SensorEvent event) {
                NativeMethods.onGyroscopeChange(
                        sessionId,
                        event.values[0],
                        event.values[1],
                        event.values[2]
                );
            }

            @Override
            public void onAccuracyChanged(Sensor sensor, int accuracy) {
            }
        };

        sensorManager.registerListener(gyroscopeListener, sensor, delay);
    }

    /**
     * Stop listening for gyroscope events.
     */
    public void stopGyroscope() {
        if (sensorManager != null && gyroscopeListener != null) {
            sensorManager.unregisterListener(gyroscopeListener);
            gyroscopeListener = null;
        }
    }

    // ==================== Cleanup ====================

    /**
     * Stop all sensor listeners. Call when session is destroyed.
     */
    public void destroy() {
        stopDeviceMotionListening();
        stopGyroscope();
    }

    // ==================== Internal ====================

    /**
     * Remap rotation matrix based on current display rotation so that
     * orientation angles are correct regardless of screen orientation.
     */
    private float[] remapForDisplay(float[] rotationMatrix) {
        int axisX = SensorManager.AXIS_X;
        int axisY = SensorManager.AXIS_Y;

        if (windowManager != null) {
            Display display = windowManager.getDefaultDisplay();
            int rotation = display.getRotation();
            switch (rotation) {
                case Surface.ROTATION_90:
                    axisX = SensorManager.AXIS_Y;
                    axisY = SensorManager.AXIS_MINUS_X;
                    break;
                case Surface.ROTATION_180:
                    axisX = SensorManager.AXIS_MINUS_X;
                    axisY = SensorManager.AXIS_MINUS_Y;
                    break;
                case Surface.ROTATION_270:
                    axisX = SensorManager.AXIS_MINUS_Y;
                    axisY = SensorManager.AXIS_X;
                    break;
                default: // ROTATION_0
                    break;
            }
        }

        float[] remapped = new float[9];
        SensorManager.remapCoordinateSystem(rotationMatrix, axisX, axisY, remapped);
        return remapped;
    }

    /**
     * @param interval "game", "ui", or "normal" (default)
     * @return SensorManager delay constant
     */
    private static int parseInterval(String interval) {
        if (interval == null) return SensorManager.SENSOR_DELAY_NORMAL;
        switch (interval) {
            case "game":
                return SensorManager.SENSOR_DELAY_GAME;
            case "ui":
                return SensorManager.SENSOR_DELAY_UI;
            default:
                return SensorManager.SENSOR_DELAY_NORMAL;
        }
    }
}
