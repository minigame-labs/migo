// Wraps a function with WeChat-style success/fail/complete callbacks + Promise return.
//
// Usage:
//   return wrapWxAsync('startDeviceMotionListening', () => op_start(interval), options);
//
// - If fn() returns a Promise, chains on it; otherwise wraps in Promise.resolve().
// - Always returns a Promise (supports both callback and await patterns).
// - On success: calls success(res), complete(res), resolves with res.
// - On failure: calls fail(res), complete(res), rejects with res.
function wrapWxAsync(apiName, fn, options) {
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

export { wrapWxAsync };
