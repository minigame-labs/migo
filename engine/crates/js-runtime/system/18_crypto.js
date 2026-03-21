// UserCryptoManager
//
// @stub All methods always call fail callback with "not supported".
// Requires platform-level crypto key management to be functional.
// getUserCryptoManager() returns a stub instance.

class UserCryptoManager {
    getLatestUserKey(options) {
        var opts = options || {};
        var res = { errMsg: 'getLatestUserKey:fail not supported' };
        if (typeof opts.fail === 'function') queueMicrotask(function () { opts.fail(res); });
        if (typeof opts.complete === 'function') queueMicrotask(function () { opts.complete(res); });
    }

    getRandomValues(options) {
        var opts = options || {};
        var res = { errMsg: 'getRandomValues:fail not supported' };
        if (typeof opts.fail === 'function') queueMicrotask(function () { opts.fail(res); });
        if (typeof opts.complete === 'function') queueMicrotask(function () { opts.complete(res); });
    }
}

function getUserCryptoManager() {
    return new UserCryptoManager();
}

export { getUserCryptoManager };
