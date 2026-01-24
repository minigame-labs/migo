
import { core, primordials } from "ext:core/mod.js";
const {
    TypeError,
    ReflectApply,
} = primordials;
const {
    getAsyncContext,
    setAsyncContext,
} = core;

function checkThis(thisArg) {
    if (thisArg !== null && thisArg !== undefined && thisArg !== globalThis) {
        throw new TypeError("Illegal invocation");
    }
}

function setTimeout(callback, timeout = 0, ...args) {
    checkThis(this);

    const unboundCallback = callback;
    const asyncContext = getAsyncContext();
    callback = () => {
        const oldContext = getAsyncContext();
        try {
            setAsyncContext(asyncContext);
            ReflectApply(unboundCallback, globalThis, args);
        } finally {
            setAsyncContext(oldContext);
        }
    };
    const id =  core.queueUserTimer(
        core.getTimerDepth() + 1,
        false,
        timeout,
        callback,
    );
    return id;
}

function setImmediate(callback, ...args) {
    const asyncContext = getAsyncContext();
    return core.queueImmediate(() => {
        const oldContext = getAsyncContext();
        try {
            setAsyncContext(asyncContext);
            return ReflectApply(callback, globalThis, args);
        } finally {
            setAsyncContext(oldContext);
        }
    });
}

function setInterval(callback, timeout = 0, ...args) {
    checkThis(this);

    const unboundCallback = callback;
    const asyncContext = getAsyncContext();
    callback = () => {
        const oldContext = getAsyncContext(asyncContext);
        try {
            setAsyncContext(asyncContext);
            ReflectApply(unboundCallback, globalThis, args);
        } finally {
            setAsyncContext(oldContext);
        }
    };

    return core.queueUserTimer(
        core.getTimerDepth() + 1,
        true,
        timeout,
        callback,
    );
}

function clearTimeout(id = 0) {
    checkThis(this);
    core.cancelTimer(id);
}

function clearInterval(id = 0) {
    checkThis(this);
    core.cancelTimer(id);
}

/**
 * Mark a timer as not blocking event loop exit.
 */
function unrefTimer(id) {
  core.unrefTimer(id);
}

/**
 * Mark a timer as blocking event loop exit.
 */
function refTimer(id) {
  core.refTimer(id);
}

export {
    setTimeout,
    clearTimeout,
    setImmediate,
    setInterval,
    clearInterval,
    unrefTimer,
    refTimer,
};