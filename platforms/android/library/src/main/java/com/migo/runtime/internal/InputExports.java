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

    /**
     * The keyboard belonging to the runtime that is running now.
     *
     * <p>Not {@code sKeyboardManagers.get}: the cache is keyed by session, so a
     * restart would otherwise hand the replacement isolate the manager the
     * retired one built. That manager stamps the generation it captured, which
     * the engine drops at dispatch — the keyboard would come up and report
     * nothing at all. A retired one is torn down here, and the caller builds a
     * fresh one against the live runtime.
     */
    private static KeyboardManager liveKeyboardManager(int sessionId) {
        return RuntimeGenerationBoundary.liveEntry(
                sKeyboardManagers, sessionId, KeyboardManager::destroy);
    }

    private static KeyboardManager getOrCreateKeyboardManager(int sessionId) {
        KeyboardManager existing = liveKeyboardManager(sessionId);
        if (existing != null) return existing;
        synchronized (sKeyboardLock) {
            existing = liveKeyboardManager(sessionId);
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
        // A retired keyboard is destroyed by this lookup rather than hidden,
        // and destroying it hides it — so the view the previous runtime put on
        // screen still goes away, and no event comes back from it.
        KeyboardManager mgr = liveKeyboardManager(sessionId);
        if (mgr != null) {
            mgr.hide();
        }
    }

    public static void keyboardUpdate(int sessionId, String value) {
        KeyboardManager mgr = liveKeyboardManager(sessionId);
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
                    CallbackCorrelation.failure(
                            CallbackCorrelation.requestIdOf(optionsJson), "scanCode", "no context"));
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
