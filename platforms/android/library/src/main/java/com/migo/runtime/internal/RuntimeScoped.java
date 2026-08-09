package com.migo.runtime.internal;

/**
 * A platform object that belongs to one runtime of one session.
 *
 * <p>Implemented by the per-session managers the JavaScript side creates on
 * demand — a keyboard, a camera, a sensor listener. Each captures a token when
 * it is built and stamps that token's generation on every event it reports, so
 * the engine can tell an event produced for a retired runtime from one produced
 * for the runtime that replaced it.
 *
 * <p>The interface exists so the caches those managers live in can be swept by
 * one implementation rather than each remembering to ask. See
 * {@link RuntimeGenerationBoundary#liveEntry}.
 *
 * @hide
 */
public interface RuntimeScoped {

    /**
     * The runtime this object was built for.
     *
     * <p>Captured once, at construction. An implementation that re-read the
     * current generation here would report itself current forever, and every
     * comparison against it would pass.
     */
    RuntimeGenerationBoundary.Token runtimeToken();
}
