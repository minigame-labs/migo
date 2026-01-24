import { core, primordials } from "ext:core/mod.js";
const { Symbol, ArrayPrototypeEvery,
    ArrayPrototypePush,
    FunctionPrototypeApply,
    SafeSet,
    SafeSetIterator,
    SafeWeakRef,
    SafeWeakSet,
    SetPrototypeAdd,
    SetPrototypeDelete,
    TypeError,
    WeakRefPrototypeDeref,
    WeakSetPrototypeAdd,
    WeakSetPrototypeHas, } = primordials;
import * as utility from "ext:host_v8_base/00_base.js";
import { clearTimeout, refTimer, unrefTimer } from "./02_timers.js";
import { DOMException } from "./01_dom_exception.js";
import {
  defineEventHandler,
  Event,
  EventTarget,
  listenerCount,
  setIsTrusted,
} from "./02_event.js";

const add = Symbol("[[add]]");
const signalAbort = Symbol("[[signalAbort]]");
const remove = Symbol("[[remove]]");
const abortReason = Symbol("[[abortReason]]");
const abortAlgos = Symbol("[[abortAlgos]]");
const dependent = Symbol("[[dependent]]");
const sourceSignals = Symbol("[[sourceSignals]]");
const dependentSignals = Symbol("[[dependentSignals]]");
const signal = Symbol("[[signal]]");
const timerId = Symbol("[[timerId]]");
const illegalConstructorKey = Symbol("illegalConstructorKey");

class AbortSignal extends EventTarget {
    [abortReason] = undefined;
    [abortAlgos] = null;
    [dependent] = false;
    [sourceSignals] = null;
    [dependentSignals] = null;
    [timerId] = null;

    static any(signals) {
        const prefix = "Failed to execute 'AbortSignal.any'";
        utility.requiredArguments(arguments.length, 1, prefix);
        return createDependentAbortSignal(signals, prefix);
    }

    static abort(reason = undefined) {
        if (reason !== undefined) {
            reason = utility.converters.any(reason);
        }
        const signal = new AbortSignal(illegalConstructorKey);
        signal[signalAbort](reason);
        return signal;
    }

    static timeout(millis) {
        const prefix = "Failed to execute 'AbortSignal.timeout'";
        utility.requiredArguments(arguments.length, 1, prefix);
        millis = utility.converters["unsigned long long"](
            millis,
            prefix,
            "Argument 1",
            {
                enforceRange: true,
            },
        );

        const signal = new AbortSignal(illegalConstructorKey);
        signal[timerId] = core.queueSystemTimer(
            undefined,
            false,
            millis,
            () => {
                clearTimeout(signal[timerId]);
                signal[timerId] = null;
                signal[signalAbort](
                    new DOMException("Signal timed out.", "TimeoutError"),
                );
            },
        );
        unrefTimer(signal[timerId]);
        return signal;
    }

    [add](algorithm) {
        if (this.aborted) {
            return;
        }
        this[abortAlgos] ??= new SafeSet();
        SetPrototypeAdd(this[abortAlgos], algorithm);
    }

    [signalAbort](
        reason = new DOMException("The signal has been aborted", "AbortError"),
    ) {
        if (this.aborted) {
            return;
        }
        this[abortReason] = reason;
        const algos = this[abortAlgos];
        this[abortAlgos] = null;

        if (listenerCount(this, "abort") > 0) {
            const event = new Event("abort");
            setIsTrusted(event, true);
            super.dispatchEvent(event);
        }
        if (algos !== null) {
            for (const algorithm of new SafeSetIterator(algos)) {
                algorithm();
            }
        }

        if (this[dependentSignals] !== null) {
            const dependentSignalArray = this[dependentSignals].toArray();
            for (let i = 0; i < dependentSignalArray.length; ++i) {
                const dependentSignal = dependentSignalArray[i];
                dependentSignal[signalAbort](reason);
            }
        }
    }

    [remove](algorithm) {
        this[abortAlgos] && SetPrototypeDelete(this[abortAlgos], algorithm);
    }

    constructor(key = null) {
        if (key !== illegalConstructorKey) {
            throw new TypeError("Illegal constructor.");
        }
        super();
    }

    get aborted() {
        console.log('abort check', this[abortReason] !== undefined);
        return this[abortReason] !== undefined;
    }

    get reason() {
        return this[abortReason];
    }

    throwIfAborted() {
        if (this[abortReason] !== undefined) {
            throw this[abortReason];
        }
    }

    addEventListener() {
        FunctionPrototypeApply(super.addEventListener, this, arguments);
        if (listenerCount(this, "abort") > 0) {
            if (this[timerId] !== null) {
                refTimer(this[timerId]);
            } else if (this[sourceSignals] !== null) {
                const sourceSignalArray = this[sourceSignals].toArray();
                for (let i = 0; i < sourceSignalArray.length; ++i) {
                    const sourceSignal = sourceSignalArray[i];
                    if (sourceSignal[timerId] !== null) {
                        refTimer(sourceSignal[timerId]);
                    }
                }
            }
        }
    }

    removeEventListener() {
        FunctionPrototypeApply(super.removeEventListener, this, arguments);
        if (listenerCount(this, "abort") === 0) {
            if (this[timerId] !== null) {
                unrefTimer(this[timerId]);
            } else if (this[sourceSignals] !== null) {
                const sourceSignalArray = this[sourceSignals].toArray();
                for (let i = 0; i < sourceSignalArray.length; ++i) {
                    const sourceSignal = sourceSignalArray[i];
                    if (sourceSignal[timerId] !== null) {
                        // Check that all dependent signals of the timer signal do not have listeners
                        if (
                            ArrayPrototypeEvery(
                                sourceSignal[dependentSignals].toArray(),
                                (dependentSignal) =>
                                    dependentSignal === this ||
                                    listenerCount(dependentSignal, "abort") === 0,
                            )
                        ) {
                            unrefTimer(sourceSignal[timerId]);
                        }
                    }
                }
            }
        }
    }
}

defineEventHandler(AbortSignal.prototype, "abort");

utility.configureInterface(AbortSignal);
const AbortSignalPrototype = AbortSignal.prototype;

class AbortController {
    [signal] = new AbortSignal(illegalConstructorKey);

    constructor() {
    }

    get signal() {
        return this[signal];
    }

    abort(reason) {
        this[signal][signalAbort](reason);
    }
}

utility.configureInterface(AbortController);
const AbortControllerPrototype = AbortController.prototype;

utility.converters.AbortSignal = utility.createInterfaceConverter(
    "AbortSignal",
    AbortSignal.prototype,
);
utility.converters["sequence<AbortSignal>"] = utility.createSequenceConverter(
    utility.converters.AbortSignal,
);

function newSignal() {
    return new AbortSignal(illegalConstructorKey);
}

class WeakRefSet {
  #weakSet = new SafeWeakSet();
  #refs = [];

  add(value) {
    if (WeakSetPrototypeHas(this.#weakSet, value)) {
      return;
    }
    WeakSetPrototypeAdd(this.#weakSet, value);
    ArrayPrototypePush(this.#refs, new SafeWeakRef(value));
  }

  has(value) {
    return WeakSetPrototypeHas(this.#weakSet, value);
  }

  toArray() {
    const ret = [];
    for (let i = 0; i < this.#refs.length; ++i) {
      const value = WeakRefPrototypeDeref(this.#refs[i]);
      if (value !== undefined) {
        ArrayPrototypePush(ret, value);
      }
    }
    return ret;
  }
}

function createDependentAbortSignal(signals, prefix) {
    signals = utility.converters["sequence<AbortSignal>"](
        signals,
        prefix,
        "Argument 1",
    );

    const resultSignal = new AbortSignal(illegalConstructorKey);
    for (let i = 0; i < signals.length; ++i) {
        const signal = signals[i];
        if (signal[abortReason] !== undefined) {
            resultSignal[abortReason] = signal[abortReason];
            return resultSignal;
        }
    }

    resultSignal[dependent] = true;
    resultSignal[sourceSignals] = new WeakRefSet();
    for (let i = 0; i < signals.length; ++i) {
        const signal = signals[i];
        if (!signal[dependent]) {
            signal[dependentSignals] ??= new WeakRefSet();
            resultSignal[sourceSignals].add(signal);
            signal[dependentSignals].add(resultSignal);
        } else {
            const sourceSignalArray = signal[sourceSignals].toArray();
            for (let j = 0; j < sourceSignalArray.length; ++j) {
                const sourceSignal = sourceSignalArray[j];

                if (resultSignal[sourceSignals].has(sourceSignal)) {
                    continue;
                }
                resultSignal[sourceSignals].add(sourceSignal);
                sourceSignal[dependentSignals].add(resultSignal);
            }
        }
    }

    return resultSignal;
}

export {
  AbortController,
  AbortSignal,
  AbortSignalPrototype,
  add,
  createDependentAbortSignal,
  newSignal,
  remove,
  signalAbort,
  timerId,
};
