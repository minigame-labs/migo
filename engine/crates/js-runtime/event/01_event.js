import { core, primordials } from "ext:core/mod.js";

const { SafeSet, SafeSetIterator, SetPrototypeDelete } = primordials;

const errorListeners = new SafeSet();

function onError(listener) {
    if (typeof listener !== 'function') {
        throw new TypeError('Error listener must be a function');
    }
    errorListeners.add(listener);
}

function offError(listener) {
    if (listener === undefined) {
        errorListeners.clear();
        return;
    }
    if (typeof listener !== 'function') {
        throw new TypeError('Error listener must be a function');
    }
    SetPrototypeDelete(errorListeners, listener);
}

function notifyErrorListeners(message) {
    const listeners = new SafeSetIterator(errorListeners);
    for (const listener of listeners) {
        try {
            listener(message);
        } catch (err) {
            console.error('Error in error listener:', err);
        }
    }
}

function formatErrorMessage(reason) {
    if (reason instanceof Error) {
        return reason.stack || reason.message || String(reason);
    }
    return String(reason);
}

function processUnhandledPromiseRejection(promise, reason) {
    const message = formatErrorMessage(reason);
    console.error('Unhandled Promise Rejection:', message);
    notifyErrorListeners(message);
    return true;
}

function processRejectionHandled(promise, reason) {
    const message = formatErrorMessage(reason);
    console.log('Promise Rejection Handled:', message);
}

function initializeEventHandlers() {
    core.setUnhandledPromiseRejectionHandler(processUnhandledPromiseRejection);
    core.setHandledPromiseRejectionHandler(processRejectionHandled);
}

export {
    onError,
    offError,
    initializeEventHandlers,
};
