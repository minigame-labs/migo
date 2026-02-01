import { core, primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { WindowGlobalScope } from "ext:runtime/98_global_scope_window.js";

const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, WindowGlobalScope);

globalThis.GameGlobal = globalThis;
globalThis.migo = globalThis;

// TODO: move to a sperate folder
const _errorListeners = [];

function onError(listener) {
    if (typeof listener === 'function') {
        _errorListeners.push(listener);
    }
}

function offError(listener) {
    if (listener === undefined) {
        _errorListeners.length = 0;
        return;
    }
    if (typeof listener === 'function') {
        const index = _errorListeners.indexOf(listener);
        if (index !== -1) {
            _errorListeners.splice(index, 1);
        }
    }
}

globalThis.onError = onError;
globalThis.offError = offError;

// -- Unhandled promise rejection handler --

core.setUnhandledPromiseRejectionHandler(processUnhandledPromiseRejection);
core.setHandledPromiseRejectionHandler(processRejectionHandled);

function processUnhandledPromiseRejection(promise, reason) {
    const message = reason instanceof Error
        ? (reason.stack || reason.message || String(reason))
        : String(reason);
    console.error(message);
    for (const listener of _errorListeners) {
        try {
            listener(message);
        } catch (_) {}
    }
    return true;
}

function processRejectionHandled(promise, reason) {
    console.log(reason);
}