// config / ready / error
//
// Lightweight JSSDK lifecycle event bus.
// - config(cfg): caches config, marks ready, fires queued ready callbacks.
// - ready(cb): if already configured, fires immediately; otherwise queues.
// - error(cb): registers global error listener; fires on config failure.

import { createListenerGroup } from "ext:host_v8_base/02_async.js";

let _configured = false;
let _configData = null;
const _readyQueue = [];
const _errorListeners = createListenerGroup('error callback');

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
    _errorListeners.on(callback);
}

// Host can call this to signal a config-level error.
function _internalTriggerJssdkError(errMsg) {
    const res = { errMsg: errMsg || 'config:fail' };
    _errorListeners.trigger(res);
}

export { config, ready, error, _internalTriggerJssdkError };
