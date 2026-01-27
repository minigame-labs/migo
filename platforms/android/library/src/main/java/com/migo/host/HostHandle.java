package com.migo.host;

import android.view.MotionEvent;
import android.view.Surface;

import com.migo.host.internal.jni.HostJNI;
import com.migo.host.internal.jni.HostNative;
import com.migo.host.internal.jni.MotionEventFlattener;
import com.migo.host.internal.runtime.HostRuntimeRegistry;

import java.nio.ByteBuffer;

public final class HostHandle {
    private final int hostId;
    private final InitOption initOption;
    private boolean destroyed = false;

    private static final int MAX_POINTERS = 10;
    private final ByteBuffer touchBuffer = ByteBuffer.allocateDirect(MAX_POINTERS * MotionEventFlattener.STRIDE_BYTES);

    HostHandle(int id, InitOption opt) {
        this.hostId = id;
        this.initOption = opt;
    }

    public boolean startGame(String codeDir, String entry) {
        ensureNotDestroyed();
        return HostNative.modMain(hostId, codeDir, entry) == 0;
    }

    public void destroy() {
        if (destroyed) return;
        HostNative.shutdown(hostId);
        HostRuntimeRegistry.unregister(hostId);
        destroyed = true;
    }

    public void onTouch(MotionEvent e) {
        int pointerCount = MotionEventFlattener.flatten(e, initOption.getDpi(), touchBuffer);
        HostJNI.onTouchEvent(hostId, e.getActionMasked(), e.getEventTime(), pointerCount, touchBuffer);
    }

    public void updateSurface(Surface surface) {
        ensureNotDestroyed();
        HostNative.updateSurface(hostId, surface);
    }

    public void onShow() {
        ensureNotDestroyed();
        HostNative.onShow(hostId);
    }

    public void onHide() {
        ensureNotDestroyed();
        HostNative.onHide(hostId);
    }

    private void ensureNotDestroyed() {
        if (destroyed) {
            throw new IllegalStateException("MiniGameHost has been destroyed.");
        }
    }
}
