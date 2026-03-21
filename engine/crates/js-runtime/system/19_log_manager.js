// LogManager / RealtimeLogManager
//
// LogManager: delegates to console (functional).
// @stub RealtimeLogManager: all methods are no-op. Requires cloud
// logging backend integration to be functional.

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
