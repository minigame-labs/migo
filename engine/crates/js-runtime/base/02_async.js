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

export { wrapAsync, promisify };
