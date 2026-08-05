package com.migo.runtime.internal;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Process-wide ownership of the device resources only one Session may hold.
 *
 * <p>One Engine may run several games at once, and each Session builds its own
 * platform managers. Camera, microphone and the Bluetooth adapter are single
 * system objects, so two Sessions previously acquired them independently and the
 * second acquirer silently broke the first: a capture session already streaming
 * would go black or corrupt with nothing to observe. Ownership is therefore
 * explicit and lives here rather than in any one Session's manager, because a
 * per-Session structure cannot arbitrate between Sessions.
 *
 * <p><b>First come, first served.</b> The incumbent keeps the resource and the
 * newcomer is refused, which is the opposite of a silent takeover. Revoking the
 * incumbent instead was rejected: arrival order is not evidence of who should own
 * the device — Migo cannot see which game the user is looking at — so preferring
 * the newcomer would break a running capture on the strength of nothing.
 *
 * <p>Audio focus is deliberately <i>not</i> arbitrated here. Android already owns
 * that decision, transfers focus to the latest requester, and tells the loser
 * through {@code AUDIOFOCUS_LOSS}; adding a second policy on top would either
 * contradict the platform or duplicate it.
 *
 * <p>Resource identity is granular rather than one key per device class. A phone
 * has several cameras and {@code CameraManager} is constructed per camera id, so
 * keying on "camera" alone would refuse two Sessions using different cameras,
 * which is a legitimate thing to do.
 *
 * <p>Ownership is a single owner per key, not a count. A Session that acquires a
 * key it already holds succeeds and does not need a matching extra release: within
 * one Session the manager owns its own lifecycle, and counting would only turn a
 * Session's own double-release bug into a leak that outlives it.
 */
public final class ExclusiveDeviceArbiter {
    /** The single microphone. */
    public static final String MICROPHONE = "microphone";

    /** The single Bluetooth adapter. */
    public static final String BLUETOOTH_ADAPTER = "bluetooth-adapter";

    /** One physical camera, identified so two Sessions may use different ones. */
    public static String camera(String cameraId) {
        return "camera:" + cameraId;
    }

    private static final Map<String, Integer> OWNERS = new ConcurrentHashMap<>();

    private ExclusiveDeviceArbiter() {}

    /**
     * Claims {@code resource} for {@code sessionId}, or reports that another
     * Session holds it.
     *
     * @return true when this Session now owns the resource, including when it
     *     already did; false when a different Session owns it.
     */
    public static boolean tryAcquire(String resource, int sessionId) {
        Objects.requireNonNull(resource, "resource");
        Integer owner = OWNERS.putIfAbsent(resource, sessionId);
        return owner == null || owner == sessionId;
    }

    /** Releases {@code resource} only if {@code sessionId} owns it. */
    public static void release(String resource, int sessionId) {
        Objects.requireNonNull(resource, "resource");
        OWNERS.remove(resource, sessionId);
    }

    /**
     * Releases everything a Session owns, for its teardown path.
     *
     * <p>Without this, a Session that fails or is destroyed while holding the
     * camera would keep every other game off it for the life of the process. The
     * teardown path must not depend on each manager having released cleanly,
     * because the reason a Session is being torn down may be that one did not.
     */
    public static void releaseAll(int sessionId) {
        List<String> owned = new ArrayList<>();
        for (Map.Entry<String, Integer> entry : OWNERS.entrySet()) {
            if (entry.getValue() == sessionId) owned.add(entry.getKey());
        }
        for (String resource : owned) {
            OWNERS.remove(resource, sessionId);
        }
    }

    /** The owning session id, or null when the resource is free. */
    static Integer ownerForTests(String resource) {
        return OWNERS.get(resource);
    }

    static void resetForTests() {
        OWNERS.clear();
    }
}
