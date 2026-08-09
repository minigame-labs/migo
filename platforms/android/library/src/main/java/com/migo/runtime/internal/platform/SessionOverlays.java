package com.migo.runtime.internal.platform;

import java.util.ArrayList;
import java.util.EnumMap;
import java.util.List;
import java.util.concurrent.ConcurrentHashMap;

import com.migo.runtime.internal.ResourceCleanup;

/**
 * What each session currently has on screen, and how to take it back.
 *
 * <p>A session shows at most one toast and one loading overlay, so the natural
 * shape is a slot per session — and the reason this class exists is that it used
 * to be a slot per <em>process</em>. One engine can run several games at once,
 * and a single {@code static View} field made them fight over it: the second
 * session's {@code showLoading} would take back the first session's spinner,
 * except that the view belongs to another Activity's decor, so the removal is a
 * no-op and the first session's overlay is orphaned on screen with nothing left
 * holding a reference to it.
 *
 * <p>It also leaked with one session. Nothing on the teardown path cleared those
 * fields, so a session closing while a toast was up left a {@code static}
 * reference to a View of a destroyed Activity — the whole window hierarchy held
 * alive until some later session happened to call hide.
 *
 * <p>Lifetime is kept here and the views are kept in the removers, so this can be
 * checked without an Activity. Every caller runs on the main looper today; the
 * per-session map is guarded anyway, because that is one line and the
 * alternative is re-deriving it every time this is touched.
 *
 * @hide
 */
final class SessionOverlays {

    /** The overlays a session can have exactly one of. */
    enum Slot {
        TOAST,
        LOADING,
    }

    private static final ConcurrentHashMap<Integer, EnumMap<Slot, Runnable>> sBySession =
            new ConcurrentHashMap<>();

    private SessionOverlays() {}

    /**
     * Record {@code remove} as the way to take back what this session has just
     * put on screen, taking back whatever occupied the slot first.
     */
    static void install(int sessionId, Slot slot, Runnable remove) {
        release(sessionId, slot);
        EnumMap<Slot, Runnable> slots =
                sBySession.computeIfAbsent(sessionId, id -> new EnumMap<>(Slot.class));
        synchronized (slots) {
            slots.put(slot, remove);
        }
    }

    /** Take back this session's {@code slot}, if it holds anything. */
    static void release(int sessionId, Slot slot) {
        EnumMap<Slot, Runnable> slots = sBySession.get(sessionId);
        if (slots == null) return;
        Runnable remove;
        synchronized (slots) {
            remove = slots.remove(slot);
        }
        // Out of the slot before it runs: removing a view can call back into
        // content, and a show that arrives during the teardown must not be
        // erased by the teardown that provoked it.
        if (remove != null) remove.run();
    }

    /**
     * Take back everything this session owns and stop tracking it.
     *
     * <p>Called from the session's terminal cleanup. Failing to take one overlay
     * back must not strand the others, which is what {@link ResourceCleanup#runAll}
     * is for.
     */
    static void releaseAll(int sessionId) {
        EnumMap<Slot, Runnable> slots = sBySession.remove(sessionId);
        if (slots == null) return;
        List<Runnable> removers;
        synchronized (slots) {
            removers = new ArrayList<>(slots.values());
            slots.clear();
        }
        ResourceCleanup.Action[] actions = new ResourceCleanup.Action[removers.size()];
        for (int i = 0; i < removers.size(); i++) {
            Runnable remove = removers.get(i);
            actions[i] = remove::run;
        }
        ResourceCleanup.runAll(actions);
    }

    /** Whether this session is tracked at all. Teardown must leave nothing behind. */
    static boolean isTracked(int sessionId) {
        return sBySession.containsKey(sessionId);
    }
}
