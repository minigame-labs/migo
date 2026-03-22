import { core, primordials } from "ext:core/mod.js";

const { SafeSet, SafeSetIterator, SetPrototypeDelete } = primordials;

const errorListeners = new SafeSet();
const unhandledRejectionListeners = new SafeSet();

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

function onUnhandledRejection(listener) {
    if (typeof listener !== 'function') {
        throw new TypeError('Listener must be a function');
    }
    unhandledRejectionListeners.add(listener);
}

function offUnhandledRejection(listener) {
    if (listener === undefined) {
        unhandledRejectionListeners.clear();
        return;
    }
    if (typeof listener !== 'function') {
        throw new TypeError('Listener must be a function');
    }
    SetPrototypeDelete(unhandledRejectionListeners, listener);
}

function processUnhandledPromiseRejection(promise, reason) {
    const message = formatErrorMessage(reason);
    console.error('Unhandled Promise Rejection:', message);

    // Notify onUnhandledRejection listeners with standard event shape
    const listeners = new SafeSetIterator(unhandledRejectionListeners);
    for (const listener of listeners) {
        try {
            listener({
                reason: reason,
                message: message,
                promise: promise,
            });
        } catch (err) {
            console.error('Error in unhandledRejection listener:', err);
        }
    }

    // Also notify onError listeners if no dedicated rejection listeners exist
    if (unhandledRejectionListeners.size === 0) {
        notifyErrorListeners(message);
    }
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
    onUnhandledRejection,
    offUnhandledRejection,
    initializeEventHandlers,
};
