package com.migo.runtime.internal;

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
