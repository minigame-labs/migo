// config / ready / error
//
// Lightweight JSSDK lifecycle event bus.
// - config(cfg): caches config, marks ready, fires queued ready callbacks.
// - ready(cb): if already configured, fires immediately; otherwise queues.
// - error(cb): registers global error listener; fires on config failure.

let _configured = false;
let _configData = null;
const _readyQueue = [];
const _errorListeners = [];

function config(options) {
    const opts = options || {};
    _configData = opts;
    _configured = true;

    // Flush ready queue
    for (let i = 0; i < _readyQueue.length; i++) {
        try { _readyQueue[i](); } catch (e) {
            console.error('ready callback error:', e);
        }
    }
    _readyQueue.length = 0;
}

function ready(callback) {
    if (typeof callback !== 'function') return;
    if (_configured) {
        try { callback(); } catch (e) {
            console.error('ready callback error:', e);
        }
    } else {
        _readyQueue.push(callback);
    }
}

function error(callback) {
    if (typeof callback !== 'function') return;
    _errorListeners.push(callback);
}

// Host can call this to signal a config-level error.
function _internalTriggerJssdkError(errMsg) {
    const res = { errMsg: errMsg || 'config:fail' };
    for (let i = 0; i < _errorListeners.length; i++) {
        try { _errorListeners[i](res); } catch (e) {
            console.error('error callback error:', e);
        }
    }
}

export { config, ready, error, _internalTriggerJssdkError };
