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
// Supports concurrent requests via Map-based tracking (requestId per call).
// settle() matches by parsed.requestId if present, otherwise settles the
// oldest pending request (FIFO fallback for backward compatibility).
//
// Usage:
//   const _loc = createDeferredApi('getLocation');
//
//   function getLocation(options = {}) {
//       return _loc.invoke(options, function (opts, requestId) {
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
// defaultTimeoutMs: auto-reject after this many ms if platform never settles.
// Pass 0 to disable timeout.  Individual calls can override via opts._timeout.
function createDeferredApi(apiName, defaultTimeoutMs) {
    var _nextId = 1;
    var _pending = new Map();
    if (defaultTimeoutMs === undefined) defaultTimeoutMs = 30000;

    function _settleEntry(entry, parsed) {
        if (entry._timer) clearTimeout(entry._timer);
        if (parsed.error) {
            var res = { errMsg: parsed.error };
            if (parsed.errCode !== undefined) res.errCode = parsed.errCode;
            if (entry.fail) entry.fail(res);
            if (entry.complete) entry.complete(res);
            entry.reject(res);
        } else {
            var res = { errMsg: apiName + ':ok' };
            var keys = Object.keys(parsed);
            for (var i = 0; i < keys.length; i++) {
                var k = keys[i];
                if (k !== 'requestId') res[k] = parsed[k];
            }
            if (entry.success) entry.success(res);
            if (entry.complete) entry.complete(res);
            entry.resolve(res);
        }
    }

    function invoke(options, executor) {
        var opts = options || {};
        var success = typeof opts.success === 'function' ? opts.success : null;
        var fail = typeof opts.fail === 'function' ? opts.fail : null;
        var complete = typeof opts.complete === 'function' ? opts.complete : null;
        var requestId = _nextId++;

        return new Promise(function (resolve, reject) {
            var pendingEntry = {
                resolve: resolve,
                reject: reject,
                success: success,
                fail: fail,
                complete: complete,
                _timer: 0,
            };
            _pending.set(requestId, pendingEntry);

            // Schedule auto-reject if the platform never settles.
            var ms = (opts && typeof opts._timeout === 'number') ? opts._timeout : defaultTimeoutMs;
            if (ms > 0) {
                pendingEntry._timer = setTimeout(function () {
                    var e = _pending.get(requestId);
                    if (!e) return;
                    _pending.delete(requestId);
                    var res = { errMsg: apiName + ':fail timeout' };
                    try { if (e.fail) e.fail(res); } catch (_) {}
                    try { if (e.complete) e.complete(res); } catch (_) {}
                    e.reject(res);
                }, ms);
            }

            try {
                executor(opts, requestId);
            } catch (e) {
                var entry = _pending.get(requestId);
                if (entry) {
                    _pending.delete(requestId);
                    if (entry._timer) clearTimeout(entry._timer);
                    var res = { errMsg: apiName + ':fail ' + e.message };
                    if (fail) fail(res);
                    if (complete) complete(res);
                    reject(res);
                }
            }
        });
    }

    function settle(resultJson) {
        var parsed;
        try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

        // Try requestId-based lookup first.
        var requestId = Number(parsed.requestId);
        if (Number.isFinite(requestId)) {
            var entry = _pending.get(requestId);
            if (entry) {
                _pending.delete(requestId);
                _settleEntry(entry, parsed);
            }
            // requestId was present but entry not found -- the request was
            // already settled (e.g. timed out).  Silently discard the stale
            // callback to avoid mis-settling a different pending request.
            return;
        }

        // Fallback: settle the oldest pending request (FIFO).
        // Only reached when the platform omits requestId entirely
        // (legacy / backward-compat path).
        var iter = _pending.keys();
        var first = iter.next();
        if (!first.done) {
            var entry = _pending.get(first.value);
            _pending.delete(first.value);
            _settleEntry(entry, parsed);
        }
    }

    return { invoke: invoke, settle: settle };
}

export { wrapAsync, promisify, createDeferredApi };
