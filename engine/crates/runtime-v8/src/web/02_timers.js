import { core, primordials } from "ext:core/mod.js";
import { op_now, op_timer_is_backgrounded } from "ext:core/ops";

const {
    MathCeil,
    MathFloor,
    MathMax,
    MathMin,
    MapPrototypeDelete,
    MapPrototypeForEach,
    MapPrototypeGet,
    MapPrototypeHas,
    MapPrototypeSet,
    Number,
    NumberIsFinite,
    RangeError,
    ReflectApply,
    SafeMap,
    TypeError,
    TypedArrayPrototypeGetBuffer,
    Uint8Array,
    Uint32Array,
} = primordials;

const {
    getAsyncContext,
    setAsyncContext,
} = core;

const MAX_LIVE_TIMERS = 1024;
const MAX_TIMER_ID = 0x7fffffff;
const MAX_DELAY_MS = 0x7fffffff;
const MAX_UNCLAMPED_NESTING_LEVEL = 5;
const MIN_NESTED_DELAY_MS = 4;

const _clockBytes = new Uint8Array(8);
const _clockWords = new Uint32Array(TypedArrayPrototypeGetBuffer(_clockBytes));
const _timers = new SafeMap();

let _backgrounded = false;
let _lifecycleInitialized = false;
let _liveTimerCount = 0;
let _nextTimerId = 1;

function checkThis(thisArg) {
    if (thisArg !== null && thisArg !== undefined && thisArg !== globalThis) {
        throw new TypeError("Illegal invocation");
    }
}

function ensureLifecycleInitialized() {
    if (_lifecycleInitialized) return;
    _backgrounded = op_timer_is_backgrounded();
    _lifecycleInitialized = true;
}

function monotonicNowMs() {
    op_now(_clockBytes);
    return _clockWords[0] * 1000 + _clockWords[1] / 1e6;
}

function normalizeDelay(value) {
    const number = Number(value);
    if (!NumberIsFinite(number) || number <= 0) return 0;
    return MathMin(MAX_DELAY_MS, MathFloor(number));
}

function allocateTimerId() {
    if (_liveTimerCount >= MAX_LIVE_TIMERS) {
        throw new RangeError(`Too many live timers (limit: ${MAX_LIVE_TIMERS})`);
    }

    for (let attempts = 0; attempts < MAX_LIVE_TIMERS + 1; attempts++) {
        const id = _nextTimerId;
        _nextTimerId = id === MAX_TIMER_ID ? 1 : id + 1;
        if (!MapPrototypeHas(_timers, id)) return id;
    }

    throw new RangeError("Unable to allocate a timer id");
}

function invokeEntry(entry) {
    const oldContext = getAsyncContext();
    try {
        setAsyncContext(entry.asyncContext);
        ReflectApply(entry.callback, globalThis, entry.args);
    } finally {
        setAsyncContext(oldContext);
    }
}

function isCurrentNative(entry, generation, nativeId) {
    return MapPrototypeGet(_timers, entry.id) === entry &&
        entry.generation === generation &&
        entry.nativeId === nativeId;
}

function applyRefState(entry) {
    if (entry.nativeId !== null && !entry.refed) {
        core.unrefTimer(entry.nativeId);
    }
}

function queueNative(entry, repeat, delayMs, logicalStartMs, onFire) {
    entry.generation++;
    const generation = entry.generation;
    const deadlineMs = logicalStartMs + delayMs;
    const nativeDelayMs = MathCeil(MathMax(0, deadlineMs - monotonicNowMs()));
    let nativeId = null;

    nativeId = core.queueUserTimer(
        entry.depth,
        repeat,
        nativeDelayMs,
        () => {
            if (!isCurrentNative(entry, generation, nativeId)) return;
            onFire(entry);
        },
    );

    entry.nativeId = nativeId;
    entry.activeSinceMs = logicalStartMs;
    entry.deadlineMs = deadlineMs;
    applyRefState(entry);
}

function armTimeout(entry, delayMs, logicalStartMs = monotonicNowMs()) {
    const scheduledDelay = MathCeil(MathMax(0, delayMs));
    entry.remainingMs = scheduledDelay;
    queueNative(entry, false, scheduledDelay, logicalStartMs, (current) => {
        current.nativeId = null;
        current.generation++;
        MapPrototypeDelete(_timers, current.id);
        _liveTimerCount--;
        invokeEntry(current);
    });
}

function armRepeatingInterval(
    entry,
    firstDelayMs,
    logicalStartMs = monotonicNowMs(),
) {
    const scheduledDelay = MathCeil(MathMax(0, firstDelayMs));
    entry.remainingMs = scheduledDelay;
    queueNative(entry, true, scheduledDelay, logicalStartMs, (current) => {
        current.remainingMs = current.periodMs;
        current.activeSinceMs = monotonicNowMs();
        current.deadlineMs = current.activeSinceMs + current.periodMs;
        invokeEntry(current);
    });
}

function armResumedInterval(entry, delayMs, logicalStartMs) {
    const scheduledDelay = MathCeil(MathMax(0, delayMs));
    entry.remainingMs = scheduledDelay;
    queueNative(entry, false, scheduledDelay, logicalStartMs, (current) => {
        current.nativeId = null;
        armRepeatingInterval(current, current.periodMs);
        invokeEntry(current);
    });
}

function cancelNative(entry) {
    if (entry.nativeId !== null) {
        core.cancelTimer(entry.nativeId);
        entry.nativeId = null;
    }
    entry.generation++;
}

function createTimer(kind, callback, delay, args) {
    ensureLifecycleInitialized();

    // The main isolate updates the JS lifecycle synchronously around app
    // callbacks, while a Worker may observe the shared host level before its
    // queued lifecycle edge is delivered. Treat either source as authoritative
    // for entering the background; the ordered show edge restores Worker timers.
    const backgrounded = _backgrounded || op_timer_is_backgrounded();
    const createdAtMs = monotonicNowMs();
    const depth = core.getTimerDepth() + 1;
    let delayMs = normalizeDelay(delay);
    if (depth > MAX_UNCLAMPED_NESTING_LEVEL && delayMs < MIN_NESTED_DELAY_MS) {
        delayMs = MIN_NESTED_DELAY_MS;
    }
    const id = allocateTimerId();
    const entry = {
        id,
        kind,
        callback,
        args,
        asyncContext: getAsyncContext(),
        depth,
        delayMs,
        periodMs: kind === "interval" ? MathMax(1, delayMs) : 0,
        remainingMs: delayMs,
        createdAtMs,
        activeSinceMs: createdAtMs,
        deadlineMs: createdAtMs + delayMs,
        nativeId: null,
        generation: 0,
        refed: true,
    };

    MapPrototypeSet(_timers, id, entry);
    _liveTimerCount++;

    if (!backgrounded) {
        try {
            if (kind === "interval") {
                armRepeatingInterval(entry, delayMs, createdAtMs);
            } else {
                armTimeout(entry, delayMs, createdAtMs);
            }
        } catch (error) {
            cancelNative(entry);
            MapPrototypeDelete(_timers, id);
            _liveTimerCount--;
            throw error;
        }
    }

    return id;
}

function setTimeout(callback, timeout = 0, ...args) {
    checkThis(this);
    return createTimer("timeout", callback, timeout, args);
}

function setImmediate(callback, ...args) {
    return createTimer("timeout", callback, 0, args);
}

function setInterval(callback, timeout = 0, ...args) {
    checkThis(this);
    return createTimer("interval", callback, timeout, args);
}

function toTimerId(value) {
    const number = Number(value);
    if (!NumberIsFinite(number)) return 0;
    return MathFloor(number);
}

function clearTimer(id) {
    const entry = MapPrototypeGet(_timers, toTimerId(id));
    if (entry === undefined) return;

    cancelNative(entry);
    MapPrototypeDelete(_timers, entry.id);
    _liveTimerCount--;
}

function clearTimeout(id = 0) {
    checkThis(this);
    clearTimer(id);
}

function clearInterval(id = 0) {
    checkThis(this);
    clearTimer(id);
}

/** Mark a timer as not blocking event loop exit. */
function unrefTimer(id) {
    const entry = MapPrototypeGet(_timers, toTimerId(id));
    if (entry === undefined || !entry.refed) return;
    entry.refed = false;
    if (entry.nativeId !== null) core.unrefTimer(entry.nativeId);
}

/** Mark a timer as blocking event loop exit. */
function refTimer(id) {
    const entry = MapPrototypeGet(_timers, toTimerId(id));
    if (entry === undefined || entry.refed) return;
    entry.refed = true;
    if (entry.nativeId !== null) core.refTimer(entry.nativeId);
}

function freezeEntry(entry, transitionMs) {
    if (entry.nativeId === null) return;
    if (entry.createdAtMs <= transitionMs && entry.activeSinceMs < transitionMs) {
        entry.remainingMs = MathMax(0, entry.deadlineMs - transitionMs);
    }
    cancelNative(entry);
}

function restoreEntry(entry, transitionMs) {
    if (entry.nativeId !== null) return;
    const logicalStartMs = MathMax(entry.createdAtMs, transitionMs);
    if (entry.kind === "interval") {
        armResumedInterval(entry, entry.remainingMs, logicalStartMs);
    } else {
        armTimeout(entry, entry.remainingMs, logicalStartMs);
    }
}

function _internalSetTimerBackgrounded(value, elapsedMicros = 0) {
    const next = !!value;
    _lifecycleInitialized = true;
    if (_backgrounded === next) return;

    const nowMs = monotonicNowMs();
    let elapsed = Number(elapsedMicros);
    if (!NumberIsFinite(elapsed) || elapsed < 0) elapsed = 0;
    const transitionMs = nowMs - elapsed / 1000;

    _backgrounded = next;
    if (next) {
        MapPrototypeForEach(_timers, (entry) => freezeEntry(entry, transitionMs));
        return;
    }

    try {
        MapPrototypeForEach(_timers, (entry) => restoreEntry(entry, transitionMs));
    } catch (error) {
        _backgrounded = true;
        MapPrototypeForEach(_timers, (entry) => freezeEntry(entry, transitionMs));
        throw error;
    }
}

export {
    setTimeout,
    clearTimeout,
    setImmediate,
    setInterval,
    clearInterval,
    unrefTimer,
    refTimer,
    _internalSetTimerBackgrounded,
};
