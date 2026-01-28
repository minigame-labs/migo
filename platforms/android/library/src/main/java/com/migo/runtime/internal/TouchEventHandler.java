package com.migo.runtime.internal;

import android.view.MotionEvent;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Handles touch event processing and dispatch to native code.
 * <p>
 * Converts Android MotionEvents to a packed binary format for efficient
 * JNI transfer.
 *
 * @hide
 */
public final class TouchEventHandler {

    /**
     * Size of a single touch point in bytes.
     * Layout: id(4) + x(4) + y(4) + force(4) + flags(4) = 20 bytes
     */
    public static final int TOUCH_POINT_SIZE = 20;

    /**
     * Maximum number of simultaneous touch points.
     */
    public static final int MAX_POINTERS = 10;

    /**
     * Flag indicating this pointer triggered the action.
     */
    public static final int FLAG_CHANGED = 1;

    private final float density;
    private final ByteBuffer buffer;

    /**
     * Create a new touch event handler.
     *
     * @param density Display density for coordinate scaling
     */
    public TouchEventHandler(float density) {
        this.density = density > 0 ? density : 1.0f;
        this.buffer = ByteBuffer.allocateDirect(MAX_POINTERS * TOUCH_POINT_SIZE);
        this.buffer.order(ByteOrder.nativeOrder());
    }

    /**
     * Dispatch a touch event to the native session.
     *
     * @param sessionId The session ID
     * @param event     The MotionEvent
     */
    public void dispatch(int sessionId, MotionEvent event) {
        if (sessionId < 0 || event == null) {
            return;
        }

        int count = flatten(event);
        NativeMethods.onTouchRaw(
                sessionId,
                event.getActionMasked(),
                event.getEventTime(),
                count,
                buffer
        );
    }

    /**
     * Flatten a MotionEvent into the internal buffer.
     *
     * @param event The MotionEvent
     * @return Number of touch points
     */
    private int flatten(MotionEvent event) {
        buffer.clear();

        final int count = Math.min(event.getPointerCount(), MAX_POINTERS);
        final int actionIndex = event.getActionIndex();

        for (int i = 0; i < count; i++) {
            buffer.putInt(event.getPointerId(i));
            buffer.putFloat(event.getX(i) / density);
            buffer.putFloat(event.getY(i) / density);
            buffer.putFloat(getPressureSafe(event, i));
            buffer.putInt(i == actionIndex ? FLAG_CHANGED : 0);
        }

        buffer.flip();
        return count;
    }

    /**
     * Safely get pressure value, handling potential exceptions.
     *
     * @param event The MotionEvent
     * @param index Pointer index
     * @return Pressure value (0.0-1.0), defaults to 1.0 on error
     */
    private static float getPressureSafe(MotionEvent event, int index) {
        try {
            float pressure = event.getPressure(index);
            return pressure > 0 ? pressure : 1.0f;
        } catch (Throwable t) {
            return 1.0f;
        }
    }
}
