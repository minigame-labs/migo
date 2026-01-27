package com.migo.host.internal.jni;

import android.view.MotionEvent;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

public final class MotionEventFlattener {

    public static final int STRIDE_BYTES = 20;
    public static final int FLAG_CHANGED = 1;

    private MotionEventFlattener() {
    }

    public static int flatten(MotionEvent e, float dpi, ByteBuffer buffer) {
        if (dpi <= 0f) dpi = 1f;

        buffer.clear();
        buffer.order(ByteOrder.nativeOrder());

        final int count = e.getPointerCount();
        final int actionIndex = e.getActionIndex();

        for (int i = 0; i < count; i++) {
            buffer.putInt(e.getPointerId(i));
            buffer.putFloat(e.getX(i) / dpi);
            buffer.putFloat(e.getY(i) / dpi);
            buffer.putFloat(getPressureSafe(e, i));
            buffer.putInt(i == actionIndex ? FLAG_CHANGED : 0);
        }

        buffer.flip();
        return count;
    }

    private static float getPressureSafe(MotionEvent e, int i) {
        try {
            return e.getPressure(i);
        } catch (Throwable t) {
            return 1.0f;
        }
    }
}
