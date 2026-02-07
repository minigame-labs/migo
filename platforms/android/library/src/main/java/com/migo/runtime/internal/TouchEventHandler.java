package com.migo.runtime.internal;

import android.view.MotionEvent;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Handles touch event processing and dispatch to native code.
 * <p>
 * Converts Android MotionEvents to a packed binary format for efficient
 * JNI transfer. Optimized for the hot path (~60-120 calls/sec during touch):
 * <ul>
 *   <li>Pre-allocated DirectByteBuffer (zero GC pressure)</li>
 *   <li>Multiply by inverse density instead of divide (faster FPU op)</li>
 *   <li>Early reject of unsupported action types</li>
 *   <li>Correct FLAG_CHANGED for multi-touch (all pointers for MOVE/CANCEL)</li>
 * </ul>
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
     * Flag indicating this pointer is in changedTouches.
     */
    public static final int FLAG_CHANGED = 1;

    // Multiply is faster than divide; computed once in constructor.
    private final float inverseDensity;
    private final ByteBuffer buffer;

    /**
     * Create a new touch event handler.
     *
     * @param density Display density for coordinate scaling (physical → CSS pixels)
     */
    public TouchEventHandler(float density) {
        this.inverseDensity = 1.0f / (density > 0 ? density : 1.0f);
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

        final int actionMasked = event.getActionMasked();

        // Early reject: only process touch actions, skip HOVER_MOVE, SCROLL, etc.
        switch (actionMasked) {
            case MotionEvent.ACTION_DOWN:
            case MotionEvent.ACTION_POINTER_DOWN:
            case MotionEvent.ACTION_MOVE:
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_POINTER_UP:
            case MotionEvent.ACTION_CANCEL:
                break;
            default:
                return;
        }

        final int count = flatten(event, actionMasked);
        NativeMethods.onTouchRaw(sessionId, actionMasked, event.getEventTime(), count, buffer);
    }

    /**
     * Flatten a MotionEvent into the pre-allocated buffer.
     *
     * @param event        The MotionEvent
     * @param actionMasked Pre-extracted masked action (avoids redundant call)
     * @return Number of touch points packed
     */
    private int flatten(MotionEvent event, int actionMasked) {
        buffer.clear();

        final int count = Math.min(event.getPointerCount(), MAX_POINTERS);
        final int actionIndex = event.getActionIndex();
        final float scale = inverseDensity;

        // For POINTER_DOWN/UP only the triggering pointer changed;
        // for DOWN, UP, MOVE, CANCEL all active pointers are considered changed.
        // This matches the Web Touch Events spec for changedTouches.
        final boolean perPointer = (actionMasked == MotionEvent.ACTION_POINTER_DOWN
                || actionMasked == MotionEvent.ACTION_POINTER_UP);

        for (int i = 0; i < count; i++) {
            buffer.putInt(event.getPointerId(i));
            buffer.putFloat(event.getX(i) * scale);
            buffer.putFloat(event.getY(i) * scale);
            float pressure = event.getPressure(i);
            buffer.putFloat(pressure > 0 ? pressure : 1.0f);
            buffer.putInt((!perPointer || i == actionIndex) ? FLAG_CHANGED : 0);
        }

        buffer.flip();
        return count;
    }
}
