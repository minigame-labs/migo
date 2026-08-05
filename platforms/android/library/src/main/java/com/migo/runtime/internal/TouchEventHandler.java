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
 * Instances are confined to Android's main thread by {@code GameSession}.
 * The direct buffer therefore has exactly one writer and needs no lock.
 *
 * @hide
 */
public final class TouchEventHandler {
    @FunctionalInterface
    interface NativeTouchSink {
        boolean send(int sessionId, int action, long time, int count, ByteBuffer buffer);
    }

    interface TouchEvent {
        int actionMasked();
        int actionIndex();
        long eventTime();
        int pointerCount();
        int pointerId(int index);
        float x(int index);
        float y(int index);
        float pressure(int index);
    }

    private static final class MotionTouchEvent implements TouchEvent {
        private MotionEvent event;

        void bind(MotionEvent event) {
            this.event = event;
        }

        void clear() {
            event = null;
        }

        @Override public int actionMasked() { return event.getActionMasked(); }
        @Override public int actionIndex() { return event.getActionIndex(); }
        @Override public long eventTime() { return event.getEventTime(); }
        @Override public int pointerCount() { return event.getPointerCount(); }
        @Override public int pointerId(int index) { return event.getPointerId(index); }
        @Override public float x(int index) { return event.getX(index); }
        @Override public float y(int index) { return event.getY(index); }
        @Override public float pressure(int index) { return event.getPressure(index); }
    }

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

    /**
     * Flag indicating this pointer left the surface (finger up / cancel), so it
     * belongs in changedTouches but must be excluded from the touches list.
     */
    public static final int FLAG_REMOVED = 2;

    // Multiply is faster than divide. Both density updates and dispatch are
    // main-thread confined, so ordinary field access is sufficient.
    private float inverseDensity;
    private final ByteBuffer buffer;
    private final NativeTouchSink nativeTouchSink;
    private final MotionTouchEvent motionTouchEvent;

    /**
     * Create a new touch event handler.
     *
     * @param density Display density for coordinate scaling (physical → CSS pixels)
     */
    public TouchEventHandler(float density) {
        this(density, NativeMethods::onTouchRaw);
    }

    TouchEventHandler(float density, NativeTouchSink nativeTouchSink) {
        if (nativeTouchSink == null) {
            throw new IllegalArgumentException("nativeTouchSink must not be null");
        }
        updateDensity(density);
        this.buffer = ByteBuffer.allocateDirect(MAX_POINTERS * TOUCH_POINT_SIZE);
        this.buffer.order(ByteOrder.nativeOrder());
        this.nativeTouchSink = nativeTouchSink;
        this.motionTouchEvent = new MotionTouchEvent();
    }

    /**
     * Update the physical-to-CSS conversion after a display/configuration move.
     * Invalid platform values fail closed to 1 rather than poisoning every
     * coordinate with NaN or infinity.
     */
    public void updateDensity(float density) {
        final float validated = density > 0.0f
                && !Float.isNaN(density)
                && !Float.isInfinite(density)
                ? density
                : 1.0f;
        inverseDensity = 1.0f / validated;
    }

    /**
     * Dispatch a touch event to the native session.
     *
     * @param sessionId The session ID
     * @param event     The MotionEvent
     */
    public boolean dispatch(int sessionId, MotionEvent event) {
        if (event == null) {
            return false;
        }
        motionTouchEvent.bind(event);
        try {
            return dispatch(sessionId, motionTouchEvent);
        } finally {
            motionTouchEvent.clear();
        }
    }

    boolean dispatch(int sessionId, TouchEvent event) {
        if (sessionId < 0 || event == null) {
            return false;
        }

        final int actionMasked = event.actionMasked();

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
                return false;
        }

        // Drop a pointer-down/up whose triggering pointer was truncated beyond
        // MAX_POINTERS: we cannot represent it in the packed slice, so emitting
        // the event would deliver empty changedTouches. (The other 10 pointers'
        // state is unchanged by this pointer's transition, so nothing is lost.)
        if (actionMasked == MotionEvent.ACTION_POINTER_DOWN
                || actionMasked == MotionEvent.ACTION_POINTER_UP) {
            if (event.actionIndex() >= MAX_POINTERS) {
                return false;
            }
        }

        final int count = flatten(event, actionMasked);
        if (count == 0) {
            return false;
        }
        return nativeTouchSink.send(sessionId, actionMasked, event.eventTime(), count, buffer);
    }

    /**
     * Flatten a MotionEvent into the pre-allocated buffer.
     *
     * @param event        The MotionEvent
     * @param actionMasked Pre-extracted masked action (avoids redundant call)
     * @return Number of touch points packed
     */
    private int flatten(TouchEvent event, int actionMasked) {
        buffer.clear();

        final int count = Math.min(event.pointerCount(), MAX_POINTERS);
        final int actionIndex = event.actionIndex();
        final float scale = inverseDensity;

        // For POINTER_DOWN/UP only the triggering pointer changed;
        // for DOWN, UP, MOVE, CANCEL all active pointers are considered changed.
        // This matches the Web Touch Events spec for changedTouches.
        final boolean perPointer = (actionMasked == MotionEvent.ACTION_POINTER_DOWN
                || actionMasked == MotionEvent.ACTION_POINTER_UP);

        // Pointers leaving the surface must be flagged so JS excludes them from
        // `touches` (they still appear in `changedTouches`): CANCEL removes every
        // pointer; UP/POINTER_UP removes only the triggering (actionIndex) one.
        final boolean cancel = (actionMasked == MotionEvent.ACTION_CANCEL);
        final boolean up = (actionMasked == MotionEvent.ACTION_UP
                || actionMasked == MotionEvent.ACTION_POINTER_UP);

        // Only the latest sample of a batched ACTION_MOVE is forwarded;
        // historical samples (event.getHistorical*) are intentionally coalesced.
        // Games sample input once per frame, so the newest position is what
        // matters, and coalescing minimizes per-event work and latency.
        for (int i = 0; i < count; i++) {
            buffer.putInt(event.pointerId(i));
            buffer.putFloat(event.x(i) * scale);
            buffer.putFloat(event.y(i) * scale);
            buffer.putFloat(TouchInputNormalizer.pressure(event.pressure(i)));
            int flags = (!perPointer || i == actionIndex) ? FLAG_CHANGED : 0;
            if (cancel || (up && i == actionIndex)) {
                flags |= FLAG_REMOVED;
            }
            buffer.putInt(flags);
        }

        buffer.flip();
        return count;
    }
}
