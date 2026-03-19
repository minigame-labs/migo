import {
    op_login,
    op_check_session,
    op_get_user_info,
    op_get_phone_number,
} from "ext:core/ops";

const noop = () => {};

let _nextRequestId = 1;
const _pendingLogin = new Map();
const _pendingCheckSession = new Map();
const _pendingUserInfo = new Map();
const _pendingPhoneNumber = new Map();

function _nextId() {
    const id = _nextRequestId;
    _nextRequestId += 1;
    return id;
}

function _safeErrorMessage(error) {
    if (!error) return "unknown error";
    if (typeof error === "string") return error;
    if (typeof error.message === "string" && error.message.length > 0) {
        return error.message;
    }
    try {
        return String(error);
    } catch (_) {
        return "unknown error";
    }
}

function _parseResultJson(resultJson) {
    if (typeof resultJson !== "string" || resultJson.length === 0) {
        return {};
    }
    try {
        const parsed = JSON.parse(resultJson);
        return parsed && typeof parsed === "object" ? parsed : {};
    } catch (_) {
        return {};
    }
}

function _toErrMsg(apiName, error) {
    const text = typeof error === "string" ? error.trim() : "";
    if (!text) {
        return `${apiName}:fail unknown error`;
    }
    if (text.startsWith(`${apiName}:fail`)) {
        return text;
    }
    return `${apiName}:fail ${text}`;
}

function _normalizeLang(value) {
    if (value === "zh_CN" || value === "zh_TW" || value === "en") {
        return value;
    }
    return "en";
}

function login(options = {}) {
    const opts = options && typeof options === "object" ? options : {};
    const requestId = _nextId();
    const success = typeof opts.success === "function" ? opts.success : noop;
    const fail = typeof opts.fail === "function" ? opts.fail : noop;
    const complete = typeof opts.complete === "function" ? opts.complete : noop;

    _pendingLogin.set(requestId, { success, fail, complete });

    const timeout = Number.isFinite(opts.timeout) && opts.timeout > 0
        ? Math.floor(opts.timeout)
        : undefined;

    try {
        op_login(JSON.stringify({ requestId, timeout }));
    } catch (error) {
        _pendingLogin.delete(requestId);
        const res = { errMsg: `login:fail ${_safeErrorMessage(error)}` };
        queueMicrotask(function () {
            fail(res);
            complete(res);
        });
    }
}

function checkSession(options = {}) {
    const opts = options && typeof options === "object" ? options : {};
    const requestId = _nextId();
    const success = typeof opts.success === "function" ? opts.success : noop;
    const fail = typeof opts.fail === "function" ? opts.fail : noop;
    const complete = typeof opts.complete === "function" ? opts.complete : noop;

    return new Promise(function (resolve, reject) {
        _pendingCheckSession.set(requestId, {
            success,
            fail,
            complete,
            resolve,
            reject,
        });

        try {
            op_check_session(JSON.stringify({ requestId }));
        } catch (error) {
            _pendingCheckSession.delete(requestId);
            const res = { errMsg: `checkSession:fail ${_safeErrorMessage(error)}` };
            queueMicrotask(function () {
                fail(res);
                complete(res);
                reject(res);
            });
        }
    });
}

function getUserInfo(options = {}) {
    const opts = options && typeof options === "object" ? options : {};
    const requestId = _nextId();
    const success = typeof opts.success === "function" ? opts.success : noop;
    const fail = typeof opts.fail === "function" ? opts.fail : noop;
    const complete = typeof opts.complete === "function" ? opts.complete : noop;

    _pendingUserInfo.set(requestId, { success, fail, complete });

    const withCredentials = opts.withCredentials === true;
    const lang = _normalizeLang(opts.lang);

    try {
        op_get_user_info(JSON.stringify({ requestId, withCredentials, lang }));
    } catch (error) {
        _pendingUserInfo.delete(requestId);
        const res = { errMsg: `getUserInfo:fail ${_safeErrorMessage(error)}` };
        queueMicrotask(function () {
            fail(res);
            complete(res);
        });
    }
}

function getPhoneNumber(options = {}) {
    const opts = options && typeof options === "object" ? options : {};
    const requestId = _nextId();
    const success = typeof opts.success === "function" ? opts.success : noop;
    const fail = typeof opts.fail === "function" ? opts.fail : noop;
    const complete = typeof opts.complete === "function" ? opts.complete : noop;

    _pendingPhoneNumber.set(requestId, { success, fail, complete });

    const isRealtime = opts.isRealtime === true;
    const phoneNumberNoQuotaToast = opts.phoneNumberNoQuotaToast !== false;

    try {
        op_get_phone_number(JSON.stringify({
            requestId,
            isRealtime,
            phoneNumberNoQuotaToast,
        }));
    } catch (error) {
        _pendingPhoneNumber.delete(requestId);
        const res = { errMsg: `getPhoneNumber:fail ${_safeErrorMessage(error)}` };
        queueMicrotask(function () {
            fail(res);
            complete(res);
        });
    }
}

function _internalOnLoginResult(resultJson) {
    const result = _parseResultJson(resultJson);
    const requestId = Number(result.requestId);
    if (!Number.isFinite(requestId)) {
        return;
    }

    const pending = _pendingLogin.get(requestId);
    if (!pending) {
        return;
    }
    _pendingLogin.delete(requestId);

    if (typeof result.error === "string" && result.error.length > 0) {
        const res = { errMsg: _toErrMsg("login", result.error) };
        if (result.errno !== undefined) {
            res.errno = result.errno;
        }
        pending.fail(res);
        pending.complete(res);
        return;
    }

    if (typeof result.code !== "string" || result.code.length === 0) {
        const res = { errMsg: "login:fail invalid code" };
        pending.fail(res);
        pending.complete(res);
        return;
    }

    const res = {
        errMsg: "login:ok",
        code: result.code,
    };
    pending.success(res);
    pending.complete(res);
}

function _internalOnCheckSessionResult(resultJson) {
    const result = _parseResultJson(resultJson);
    const requestId = Number(result.requestId);
    if (!Number.isFinite(requestId)) {
        return;
    }

    const pending = _pendingCheckSession.get(requestId);
    if (!pending) {
        return;
    }
    _pendingCheckSession.delete(requestId);

    if (typeof result.error === "string" && result.error.length > 0) {
        const res = { errMsg: _toErrMsg("checkSession", result.error) };
        if (result.errno !== undefined) {
            res.errno = result.errno;
        }
        pending.fail(res);
        pending.complete(res);
        pending.reject(res);
        return;
    }

    const res = { errMsg: "checkSession:ok" };
    pending.success(res);
    pending.complete(res);
    pending.resolve(res);
}

function _internalOnGetUserInfoResult(resultJson) {
    const result = _parseResultJson(resultJson);
    const requestId = Number(result.requestId);
    if (!Number.isFinite(requestId)) {
        return;
    }

    const pending = _pendingUserInfo.get(requestId);
    if (!pending) {
        return;
    }
    _pendingUserInfo.delete(requestId);

    if (typeof result.error === "string" && result.error.length > 0) {
        const res = { errMsg: _toErrMsg("getUserInfo", result.error) };
        pending.fail(res);
        pending.complete(res);
        return;
    }

    if (!result.userInfo || typeof result.userInfo !== "object") {
        const res = { errMsg: "getUserInfo:fail invalid userInfo" };
        pending.fail(res);
        pending.complete(res);
        return;
    }

    const res = {
        errMsg: "getUserInfo:ok",
        userInfo: result.userInfo,
    };

    if (typeof result.rawData === "string") {
        res.rawData = result.rawData;
    }
    if (typeof result.signature === "string") {
        res.signature = result.signature;
    }
    if (typeof result.encryptedData === "string") {
        res.encryptedData = result.encryptedData;
    }
    if (typeof result.iv === "string") {
        res.iv = result.iv;
    }
    if (typeof result.cloudID === "string") {
        res.cloudID = result.cloudID;
    }

    pending.success(res);
    pending.complete(res);
}

function _internalOnGetPhoneNumberResult(resultJson) {
    const result = _parseResultJson(resultJson);
    const requestId = Number(result.requestId);
    if (!Number.isFinite(requestId)) {
        return;
    }

    const pending = _pendingPhoneNumber.get(requestId);
    if (!pending) {
        return;
    }
    _pendingPhoneNumber.delete(requestId);

    if (typeof result.error === "string" && result.error.length > 0) {
        const res = { errMsg: _toErrMsg("getPhoneNumber", result.error) };
        if (result.errno !== undefined) {
            res.errno = result.errno;
        }
        pending.fail(res);
        pending.complete(res);
        return;
    }

    if (typeof result.code !== "string" || result.code.length === 0) {
        const res = { errMsg: "getPhoneNumber:fail invalid code" };
        pending.fail(res);
        pending.complete(res);
        return;
    }

    const res = {
        errMsg: "getPhoneNumber:ok",
        code: result.code,
    };
    pending.success(res);
    pending.complete(res);
}

export {
    login,
    checkSession,
    getUserInfo,
    getPhoneNumber,
    _internalOnLoginResult,
    _internalOnCheckSessionResult,
    _internalOnGetUserInfoResult,
    _internalOnGetPhoneNumberResult,
};
