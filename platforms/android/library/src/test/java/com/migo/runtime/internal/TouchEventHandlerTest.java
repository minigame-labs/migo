package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import android.view.MotionEvent;

import org.junit.Test;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;

public final class TouchEventHandlerTest {
    private static final class Event implements TouchEventHandler.TouchEvent {
        int action = MotionEvent.ACTION_MOVE;
        int actionIndex;
        long time = 10;
        int count = 1;

        @Override public int actionMasked() { return action; }
        @Override public int actionIndex() { return actionIndex; }
        @Override public long eventTime() { return time; }
        @Override public int pointerCount() { return count; }
        @Override public int pointerId(int index) { return 100 + index; }
        @Override public float x(int index) { return 20.0f + index; }
        @Override public float y(int index) { return 40.0f + index; }
        @Override public float pressure(int index) { return 0.5f; }
    }

    /** The x of the first packed pointer, in CSS pixels, as native would read it. */
    private static float firstPackedX(float density, Event event) {
        final float[] packed = new float[1];
        TouchEventHandler handler = new TouchEventHandler(density,
                (session, action, time, count, buffer) -> {
                    // Layout per pointer: id (int), x, y, pressure (floats).
                    packed[0] = buffer.getFloat(Integer.BYTES);
                    return true;
                });
        assertTrue(handler.dispatch(7, event));
        return packed[0];
    }

    /**
     * A density the platform cannot mean fails closed to 1, and the coordinates prove it.
     *
     * The same property the engine enforces for host pixel ratios, on the Java side of
     * the boundary: zero, negative, NaN and both infinities would otherwise poison every
     * coordinate -- 1/0 is infinity, 1/NaN is NaN, and a negative scale mirrors the
     * screen. Mutation testing negated each clause of the validation and killed nothing,
     * because the suite asserted that dispatch succeeded and never what it packed.
     *
     * A valid density is asserted in the same test, since a validator that fell back to 1
     * for everything would satisfy the invalid cases alone.
     */
    @Test
    public void anUnusableDensityFallsBackToOneRatherThanPoisoningCoordinates() {
        Event event = new Event();

        // x(0) is 20 physical pixels; at density 2 that is 10 CSS pixels.
        assertEquals(10.0f, firstPackedX(2.0f, event), 0.0001f);

        for (float density : new float[] {
                0.0f, -1.0f, -2.5f, Float.NaN,
                Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY}) {
            assertEquals(
                    "density " + density + " must fall back to 1",
                    20.0f,
                    firstPackedX(density, event),
                    0.0001f);
        }
    }

    /**
     * The fallback also applies to a density that arrives later, not only at construction:
     * a configuration change is exactly when a platform reports something unusable.
     */
    @Test
    public void aLaterUnusableDensityDoesNotDisturbTheLastGoodScale() {
        Event event = new Event();
        final float[] packed = new float[1];
        TouchEventHandler handler = new TouchEventHandler(2.0f,
                (session, action, time, count, buffer) -> {
                    packed[0] = buffer.getFloat(Integer.BYTES);
                    return true;
                });

        assertTrue(handler.dispatch(7, event));
        assertEquals(10.0f, packed[0], 0.0001f);

        handler.updateDensity(Float.NaN);
        assertTrue(handler.dispatch(7, event));
        assertEquals("an unusable update falls back to 1, not to NaN", 20.0f, packed[0], 0.0001f);

        handler.updateDensity(4.0f);
        assertTrue(handler.dispatch(7, event));
        assertEquals(5.0f, packed[0], 0.0001f);
    }

    @Test
    public void returnsTheNativeAcceptanceResult() {
        Event event = new Event();
        TouchEventHandler accepted = new TouchEventHandler(2.0f,
                (session, action, time, count, buffer) -> true);
        TouchEventHandler refused = new TouchEventHandler(2.0f,
                (session, action, time, count, buffer) -> false);

        assertTrue(accepted.dispatch(7, event));
        assertFalse(refused.dispatch(7, event));
    }

    @Test
    public void rejectsInvalidOrUnrepresentableEventsBeforeNative() {
        final int[] sends = {0};
        TouchEventHandler handler = new TouchEventHandler(1.0f,
                (session, action, time, count, buffer) -> {
                    sends[0]++;
                    return true;
                });
        Event event = new Event();

        assertFalse(handler.dispatch(-1, event));
        assertFalse(handler.dispatch(1, (TouchEventHandler.TouchEvent) null));

        event.action = MotionEvent.ACTION_HOVER_MOVE;
        assertFalse(handler.dispatch(1, event));

        event.action = MotionEvent.ACTION_POINTER_UP;
        event.actionIndex = TouchEventHandler.MAX_POINTERS;
        event.count = TouchEventHandler.MAX_POINTERS + 1;
        assertFalse(handler.dispatch(1, event));

        event.action = MotionEvent.ACTION_MOVE;
        event.actionIndex = 0;
        event.count = 0;
        assertFalse(handler.dispatch(1, event));
        assertTrue("native sink must not see rejected input", sends[0] == 0);
    }

    @Test
    public void reusesOneDirectBufferAcrossMoves() {
        List<ByteBuffer> buffers = new ArrayList<>();
        TouchEventHandler handler = new TouchEventHandler(2.0f,
                (session, action, time, count, buffer) -> {
                    assertTrue(buffer.isDirect());
                    buffers.add(buffer);
                    return true;
                });
        Event event = new Event();

        assertTrue(handler.dispatch(1, event));
        event.time++;
        assertTrue(handler.dispatch(1, event));

        assertSame(buffers.get(0), buffers.get(1));
    }
}
