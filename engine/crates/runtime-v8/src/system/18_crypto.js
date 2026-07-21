// UserCryptoManager
//
// @stub All methods always call fail callback with "not supported".
// Requires platform-level crypto key management to be functional.
// getUserCryptoManager() returns a stub instance.

import { wrapAsync } from "ext:host_v8_base/02_async.js";

class UserCryptoManager {
    getLatestUserKey(options) {
        return wrapAsync('getLatestUserKey', function () {
            throw new Error('not supported');
        }, options);
    }

    getRandomValues(options) {
        return wrapAsync('getRandomValues', function () {
            throw new Error('not supported');
        }, options);
    }
}

function getUserCryptoManager() {
    return new UserCryptoManager();
}

export { getUserCryptoManager };
