// ---------------------------------------------------------------------------
// MigoPerf: Unified performance profiling for the Migo runtime.
//
// All logging uses the [MigoPerf] prefix for easy logcat filtering:
//   adb logcat | grep "\[MigoPerf\]"
//
// Enable:  migo._perf.enable()          // default thresholds
//          migo._perf.enable({ sync: 3, async: 100, deferred: 200, frame: 10 })
// Disable: migo._perf.disable()
// Check:   migo._perf.enabled            // boolean
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

function _perfEnable(options) {
    _perf.enabled = true;
    if (options && typeof options === 'object') {
        if (typeof options.sync === 'number') _perf.syncMs = options.sync;
        if (typeof options.async === 'number') _perf.asyncMs = options.async;
        if (typeof options.deferred === 'number') _perf.deferredMs = options.deferred;
        if (typeof options.frame === 'number') _perf.frameMs = options.frame;
    }
    console.log('[MigoPerf] enabled: sync>' + _perf.syncMs + 'ms async>' +
        _perf.asyncMs + 'ms deferred>' + _perf.deferredMs + 'ms frame>' + _perf.frameMs + 'ms');
}

function _perfDisable() {
    _perf.enabled = false;
    console.log('[MigoPerf] disabled');
}

export { _perf, _perfEnable, _perfDisable };
