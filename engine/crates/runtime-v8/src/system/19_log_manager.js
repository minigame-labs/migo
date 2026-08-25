// LogManager / RealtimeLogManager
//
// LogManager: delegates to console (functional).
// @stub getRealtimeLogManager returns a RealtimeLogManager whose methods are
// all no-op; it needs a cloud logging backend to do anything. The marker names
// the *published* entry point rather than the class, because that is the name
// content writes and the name scripts/dump-stub-surface.sh can attribute -- a
// marker naming only the type is one nothing can resolve, which is exactly what
// that script reported when it was first pointed at this file.

class LogManager {
    debug() { console.debug.apply(console, arguments); }
    info() { console.info.apply(console, arguments); }
    log() { console.log.apply(console, arguments); }
    warn() { console.warn.apply(console, arguments); }
}

class RealtimeLogManager {
    debug() {}
    info() {}
    warn() {}
    error() {}
    setFilterMsg(msg) {}
    addFilterMsg(msg) {}
}

function getLogManager() {
    return new LogManager();
}

function getRealtimeLogManager() {
    return new RealtimeLogManager();
}

export { getLogManager, getRealtimeLogManager };
