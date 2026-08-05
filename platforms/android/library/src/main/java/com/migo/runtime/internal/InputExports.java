package com.migo.runtime.internal;

import android.app.Activity;

import com.migo.runtime.internal.platform.KeyboardManager;
import com.migo.runtime.internal.platform.ScanCodeManager;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Domain class for per-session keyboard and scan code manager delegation.
 *
 * @hide
 */
public final class InputExports {

    private InputExports() {}

    private static final Object sKeyboardLock = new Object();
    private static final Object sScanCodeLock = new Object();

    private static final ConcurrentHashMap<Integer, KeyboardManager> sKeyboardManagers =
            new ConcurrentHashMap<>();

    private static final ConcurrentHashMap<Integer, ScanCodeManager> sScanCodeManagers =
            new ConcurrentHashMap<>();

    // ==================== Keyboard ====================

    private static KeyboardManager getOrCreateKeyboardManager(int sessionId) {
        KeyboardManager existing = sKeyboardManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sKeyboardLock) {
            existing = sKeyboardManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            KeyboardManager mgr = new KeyboardManager(sessionId, activity);
            sKeyboardManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void keyboardShow(int sessionId, String optionsJson) {
        KeyboardManager mgr = getOrCreateKeyboardManager(sessionId);
        if (mgr == null) return;
        mgr.show(optionsJson);
    }

    public static void keyboardHide(int sessionId) {
        KeyboardManager mgr = sKeyboardManagers.get(sessionId);
        if (mgr != null) {
            mgr.hide();
        }
    }

    public static void keyboardUpdate(int sessionId, String value) {
        KeyboardManager mgr = sKeyboardManagers.get(sessionId);
        if (mgr != null) {
            mgr.updateValue(value);
        }
    }

    public static void destroyKeyboardManager(int sessionId) {
        ResourceCleanup.destroyMatching(
                sKeyboardManagers,
                id -> id == sessionId,
                KeyboardManager::destroy);
    }

    // ==================== Scan Code ====================

    private static ScanCodeManager getOrCreateScanCodeManager(int sessionId) {
        ScanCodeManager existing = sScanCodeManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sScanCodeLock) {
            existing = sScanCodeManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            ScanCodeManager mgr = new ScanCodeManager(sessionId, activity);
            sScanCodeManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void scanCode(int sessionId, String optionsJson) {
        ScanCodeManager mgr = getOrCreateScanCodeManager(sessionId);
        if (mgr == null) {
            NativeMethods.onScanCodeResult(sessionId,
                    "{\"error\":\"scanCode:fail no context\"}");
            return;
        }
        mgr.scanCode(optionsJson);
    }

    public static void destroyScanCodeManager(int sessionId) {
        ResourceCleanup.destroyMatching(
                sScanCodeManagers,
                id -> id == sessionId,
                ScanCodeManager::destroy);
    }

    // ==================== Bulk Destroy ====================

    public static void destroyAll(int sessionId) {
        ResourceCleanup.runAll(
                () -> destroyKeyboardManager(sessionId),
                () -> destroyScanCodeManager(sessionId));
    }
}
