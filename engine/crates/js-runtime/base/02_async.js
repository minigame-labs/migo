// Wraps a function with success/fail/complete callbacks + Promise return.
//
// Usage:
//   return wrapAsync('startDeviceMotionListening', () => op_start(interval), options);
//
// - If fn() returns a Promise, chains on it; otherwise wraps in Promise.resolve().
// - Always returns a Promise (supports both callback and await patterns).
// - On success: calls success(res), complete(res), resolves with res.
// - On failure: calls fail(res), complete(res), rejects with res.
function wrapAsync(apiName, fn, options) {
    const { success, fail, complete } = options || {};
    try {
        const result = fn();
        const p = (result instanceof Promise) ? result : Promise.resolve(result);
        return p.then(function (value) {
            const res = (typeof value === 'object' && value !== null)
                ? { errMsg: apiName + ':ok', ...value }
                : { errMsg: apiName + ':ok' };
            if (typeof success === 'function') success(res);
            if (typeof complete === 'function') complete(res);
            return res;
        }).catch(function (e) {
            const res = { errMsg: apiName + ':fail ' + (e.message || String(e)) };
            if (typeof fail === 'function') fail(res);
            if (typeof complete === 'function') complete(res);
            throw res;
        });
    } catch (e) {
        const res = { errMsg: apiName + ':fail ' + (e.message || String(e)) };
        if (typeof fail === 'function') queueMicrotask(function () { fail(res); });
        if (typeof complete === 'function') queueMicrotask(function () { complete(res); });
        return Promise.reject(res);
    }
}

// Higher-level factory: creates a callback+Promise async API from an executor.
//
// Usage:
//   const getStorage = promisify("getStorage", (opts) => {
//       return { data: getStorageSync(opts.key) };
//   });
//
//   // Callback style
//   getStorage({ key: "k", success(res) { console.log(res.data); } });
//
//   // Promise style
//   const res = await getStorage({ key: "k" });
//
// The executor receives the options object (minus callbacks) and should:
//   - Return nothing for void APIs (setStorage, removeStorage, ...)
//   - Return an object whose fields are merged into the success result
//   - Throw on error
//   - Return a Promise for truly async work
function promisify(apiName, executor) {
    return function (options) {
        return wrapAsync(apiName, function () {
            return executor(options || {});
        }, options);
    };
}

// Factory for Mode C async APIs (platform callback via EvalScript).
//
// Creates a deferred Promise+callback pair for APIs where the op only fires
// an async platform request and the result arrives later through a separate
// `_internalOn*Result` callback.
//
// Usage:
//   const _loc = createDeferredApi('getLocation');
//
//   function getLocation(options = {}) {
//       return _loc.invoke(options, function (opts) {
//           op_get_location(JSON.stringify({ type: opts.type || 'wgs84' }));
//       });
//   }
//   function _internalOnLocationResult(json) { _loc.settle(json); }
//
// - invoke(options, executor): stores callbacks + resolve/reject, calls executor, returns Promise
// - settle(resultJson): parses JSON, resolves/rejects, fires success/fail/complete
//
// Both callback and Promise styles are supported:
//   getLocation({ success(res) { ... } });        // callback
//   const res = await getLocation({ type: 'gcj02' }); // promise
function createDeferredApi(apiName) {
    let _resolve = null;
    let _reject = null;
    let _success = null;
    let _fail = null;
    let _complete = null;

    function _clear() {
        _resolve = _reject = _success = _fail = _complete = null;
    }

    function invoke(options, executor) {
        const { success, fail, complete } = options || {};
        return new Promise(function (resolve, reject) {
            _resolve = resolve;
            _reject = reject;
            _success = typeof success === 'function' ? success : null;
            _fail = typeof fail === 'function' ? fail : null;
            _complete = typeof complete === 'function' ? complete : null;
            try {
                executor(options || {});
            } catch (e) {
                var res = { errMsg: apiName + ':fail ' + e.message };
                _clear();
                if (typeof fail === 'function') fail(res);
                if (typeof complete === 'function') complete(res);
                reject(res);
            }
        });
    }

    function settle(resultJson) {
        var parsed;
        try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

        var s = _success, f = _fail, c = _complete;
        var resolve = _resolve, reject = _reject;
        _clear();

        if (parsed.error) {
            var res = { errMsg: parsed.error };
            if (f) f(res);
            if (c) c(res);
            if (reject) reject(res);
        } else {
            var res = { errMsg: apiName + ':ok', ...parsed };
            if (s) s(res);
            if (c) c(res);
            if (resolve) resolve(res);
        }
    }

    return { invoke: invoke, settle: settle };
}

export { wrapAsync, promisify, createDeferredApi };
