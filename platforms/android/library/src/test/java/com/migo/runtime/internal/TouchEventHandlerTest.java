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

    /** One packed pointer, as native reads it: id, CSS x, CSS y, pressure, flags. */
    private static final class Packed {
        int id;
        float x;
        float y;
        float pressure;
        int flags;
    }

    /** Dispatch and decode every pointer the handler packed. */
    private static List<Packed> pack(float density, Event event) {
        final List<Packed> packed = new ArrayList<>();
        TouchEventHandler handler = new TouchEventHandler(density,
                (session, action, time, count, buffer) -> {
                    for (int i = 0; i < count; i++) {
                        final int base = i * TouchEventHandler.TOUCH_POINT_SIZE;
                        Packed point = new Packed();
                        point.id = buffer.getInt(base);
                        point.x = buffer.getFloat(base + 4);
                        point.y = buffer.getFloat(base + 8);
                        point.pressure = buffer.getFloat(base + 12);
                        point.flags = buffer.getInt(base + 16);
                        packed.add(point);
                    }
                    return true;
                });
        assertTrue(handler.dispatch(7, event));
        return packed;
    }

    private static String flagsOf(List<Packed> packed) {
        StringBuilder text = new StringBuilder();
        for (Packed point : packed) {
            if (text.length() > 0) text.append(',');
            text.append((point.flags & TouchEventHandler.FLAG_CHANGED) != 0 ? "C" : "-");
            text.append((point.flags & TouchEventHandler.FLAG_REMOVED) != 0 ? "R" : "-");
        }
        return text.toString();
    }

    /**
     * The changedTouches and touches semantics of the Web Touch Events spec, per action.
     *
     * This is the contract the packed flags exist for: `changedTouches` is what the
     * transition affected, and a pointer leaving the surface must be flagged so JS drops
     * it from `touches` while keeping it in `changedTouches`. Mutation testing negated
     * every clause deciding those flags -- what makes an action per-pointer, what counts
     * as a cancel, what counts as an up -- and killed nothing, because the suite asserted
     * that dispatch happened and never what it packed. A wrong flag here is a game whose
     * touch stays stuck down, or one that never sees a tap.
     *
     * All five actions in one test on purpose: the flags are decided by three booleans
     * over one action value, so an assertion about a single action cannot tell a correct
     * rule from one that happens to agree there.
     */
    @Test
    public void packedFlagsFollowTheWebTouchEventsContract() {
        Event event = new Event();
        event.count = 3;

        event.action = MotionEvent.ACTION_MOVE;
        event.actionIndex = 0;
        assertEquals("a move changes every pointer and removes none",
                "C-,C-,C-", flagsOf(pack(1.0f, event)));

        event.action = MotionEvent.ACTION_DOWN;
        assertEquals("a first down changes every pointer", "C-,C-,C-",
                flagsOf(pack(1.0f, event)));

        event.action = MotionEvent.ACTION_POINTER_DOWN;
        event.actionIndex = 1;
        assertEquals("only the arriving pointer changed", "--,C-,--",
                flagsOf(pack(1.0f, event)));

        event.action = MotionEvent.ACTION_POINTER_UP;
        event.actionIndex = 1;
        assertEquals("only the leaving pointer changed, and it is removed", "--,CR,--",
                flagsOf(pack(1.0f, event)));

        event.action = MotionEvent.ACTION_UP;
        event.actionIndex = 0;
        assertEquals("a last up changes every pointer and removes the triggering one",
                "CR,C-,C-", flagsOf(pack(1.0f, event)));

        event.action = MotionEvent.ACTION_CANCEL;
        assertEquals("a cancel removes every pointer", "CR,CR,CR",
                flagsOf(pack(1.0f, event)));
    }

    /**
     * Both coordinates are scaled, and exactly the pointers reported are packed.
     *
     * y had no assertion at all -- replacing its multiply with a divide survived, while
     * the same mutation on x did not. And the loop bound: one pointer too many would read
     * an unreported index and hand native a coordinate from nowhere.
     */
    @Test
    public void everyReportedPointerIsPackedOnceInCssPixels() {
        Event event = new Event();
        event.count = 2;
        event.action = MotionEvent.ACTION_MOVE;

        List<Packed> packed = pack(2.0f, event);
        assertEquals("exactly the reported pointers are packed", 2, packed.size());

        // Event reports x = 20 + index, y = 40 + index physical; density 2 halves both.
        assertEquals(100, packed.get(0).id);
        assertEquals(10.0f, packed.get(0).x, 0.0001f);
        assertEquals(20.0f, packed.get(0).y, 0.0001f);
        assertEquals(101, packed.get(1).id);
        assertEquals(10.5f, packed.get(1).x, 0.0001f);
        assertEquals(20.5f, packed.get(1).y, 0.0001f);
    }

    /**
     * Session id 0 is a valid session, and a pointer-down beyond the packable slice is
     * dropped rather than delivered with empty changedTouches.
     */
    @Test
    public void sessionZeroIsAcceptedAndAnUnrepresentableTransitionIsNot() {
        final int[] sends = {0};
        TouchEventHandler handler = new TouchEventHandler(1.0f,
                (session, action, time, count, buffer) -> {
                    sends[0]++;
                    return true;
                });
        Event event = new Event();

        assertTrue("session id 0 is a session", handler.dispatch(0, event));
        assertEquals(1, sends[0]);

        event.action = MotionEvent.ACTION_POINTER_DOWN;
        event.count = TouchEventHandler.MAX_POINTERS + 1;
        event.actionIndex = TouchEventHandler.MAX_POINTERS - 1;
        assertTrue("a triggering pointer inside the slice is still delivered",
                handler.dispatch(0, event));
        assertEquals(2, sends[0]);

        event.actionIndex = TouchEventHandler.MAX_POINTERS;
        assertFalse("a triggering pointer beyond the slice is dropped",
                handler.dispatch(0, event));
        assertEquals("a dropped event must not reach native", 2, sends[0]);
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
