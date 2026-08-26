// ---------------------------------------------------------------------------
// Internal performance thresholds used by the async wrappers. Profiling is
// disabled in the content runtime; enabling it belongs to a native diagnostic
// build rather than a game-visible global API.
//
// All logging uses the [MigoPerf] prefix for easy logcat filtering:
//   adb logcat | grep "\[MigoPerf\]"
//
// When enabled, logs:
//   [MigoPerf][Sync]     apiName: 8.3ms            - sync op blocked V8
//   [MigoPerf][Async]    apiName: 2340ms            - async op end-to-end
//   [MigoPerf][Deferred] apiName: 1523ms            - platform callback round-trip
//   [MigoPerf][Frame]    total=14.2ms js=11.8ms flush=2.4ms  - per-frame breakdown
//
// Zero overhead when disabled: all hot paths check a single boolean.
// ---------------------------------------------------------------------------

const _perf = {
    enabled: false,
    // Thresholds in ms. Only log calls exceeding these.
    syncMs: 2,        // sync op blocking V8 (wrapAsync fn() call)
    asyncMs: 50,      // async op end-to-end (wrapAsync Promise resolve)
    deferredMs: 100,  // Mode C platform round-trip (createDeferredApi settle)
    frameMs: 8,       // per-frame total (RAF loop)
};

export { _perf };
