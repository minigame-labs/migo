import { primordials } from "ext:core/mod.js";
import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { WindowGlobalScope } from "ext:runtime/98_global_scope_window.js";
import { initializeEventHandlers } from "ext:host_v8_event/01_event.js";

const { ObjectDefineProperties, ObjectDefineProperty, ObjectFreeze } = primordials;

ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, WindowGlobalScope);

globalThis.GameGlobal = globalThis;
globalThis.migo = globalThis;

const _windowMetricsCache = {
    windowWidth: 0,
    windowHeight: 0,
    screenWidth: 0,
    screenHeight: 0,
    pixelRatio: 1,
};

function _readWindowMetrics() {
    try {
        // getWindowInfo is provided by host_v8_system (api-connectivity feature).
        // When the feature is disabled it will not exist on globalThis, so we
        // guard the call and fall back to the last cached values.
        var _getWinInfo = globalThis.getWindowInfo;
        if (typeof _getWinInfo !== 'function') return _windowMetricsCache;
        const info = _getWinInfo();
        _windowMetricsCache.windowWidth = info.windowWidth || 0;
        _windowMetricsCache.windowHeight = info.windowHeight || 0;
        _windowMetricsCache.screenWidth = info.screenWidth || _windowMetricsCache.windowWidth;
        _windowMetricsCache.screenHeight = info.screenHeight || _windowMetricsCache.windowHeight;
        _windowMetricsCache.pixelRatio = info.pixelRatio || 1;
    } catch (_) {
        // keep last known values
    }
    return _windowMetricsCache;
}

function _defineGlobalGetter(name, getter) {
    try {
        ObjectDefineProperty(globalThis, name, {
            configurable: true,
            enumerable: true,
            get: getter,
        });
    } catch (_) {
        // ignore if property is non-configurable in this runtime
    }
}

const _screen = {};
ObjectDefineProperty(_screen, "width", {
    configurable: true,
    enumerable: true,
    get() { return _readWindowMetrics().screenWidth; },
});
ObjectDefineProperty(_screen, "height", {
    configurable: true,
    enumerable: true,
    get() { return _readWindowMetrics().screenHeight; },
});
ObjectDefineProperty(_screen, "availWidth", {
    configurable: true,
    enumerable: true,
    get() { return _readWindowMetrics().windowWidth; },
});
ObjectDefineProperty(_screen, "availHeight", {
    configurable: true,
    enumerable: true,
    get() { return _readWindowMetrics().windowHeight; },
});
ObjectFreeze(_screen);

try {
    ObjectDefineProperty(globalThis, "window", {
        configurable: true,
        enumerable: true,
        value: globalThis,
    });
} catch (_) {
    // ignore if window is already fixed by runtime
}

_defineGlobalGetter("innerWidth", function () { return _readWindowMetrics().windowWidth; });
_defineGlobalGetter("innerHeight", function () { return _readWindowMetrics().windowHeight; });
_defineGlobalGetter("outerWidth", function () { return _readWindowMetrics().windowWidth; });
_defineGlobalGetter("outerHeight", function () { return _readWindowMetrics().windowHeight; });
_defineGlobalGetter("screenWidth", function () { return _readWindowMetrics().screenWidth; });
_defineGlobalGetter("screenHeight", function () { return _readWindowMetrics().screenHeight; });
_defineGlobalGetter("devicePixelRatio", function () { return _readWindowMetrics().pixelRatio; });
_defineGlobalGetter("screen", function () { return _screen; });

let ccSettingsValue;
ObjectDefineProperty(globalThis, "_CCSettings", {
    configurable: true,
    enumerable: true,
    get() {
        return ccSettingsValue;
    },
    set(value) {
        ccSettingsValue = value;
        if (!value || typeof value !== "object") return;

        const orientation = value.orientation;
        if (orientation !== "landscape" && orientation !== "portrait") return;

        const setOrientation = globalThis.setDeviceOrientation;
        if (typeof setOrientation !== "function") return;

        try {
            const ret = setOrientation({ value: orientation });
            if (ret && typeof ret.catch === "function") {
                ret.catch(() => {});
            }
        } catch (_) {
            // ignore
        }
    },
});

// Initialize event handlers
initializeEventHandlers();
