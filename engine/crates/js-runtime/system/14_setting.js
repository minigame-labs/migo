// getSetting / authorize / openSetting
//
// Maintains an in-memory authSetting map. getSetting and authorize are pure JS.
// openSetting delegates to the host via op (Mode C) so the native settings UI
// can be shown; falls back to returning current state if no host op available.

import { op_open_setting } from "ext:core/ops";
import { wrapAsync, createDeferredApi } from "ext:host_v8_base/02_async.js";

// ---- authorisation state (scope -> boolean) --------------------------------

const _authSetting = {
    "scope.userInfo": true,
    "scope.userLocation": true,
    "scope.userLocationBackground": false,
    "scope.address": true,
    "scope.invoiceTitle": true,
    "scope.invoice": true,
    "scope.werun": true,
    "scope.record": true,
    "scope.writePhotosAlbum": true,
    "scope.camera": true,
    "scope.bluetooth": true,
    "scope.addPhoneContact": true,
    "scope.addPhoneCalendar": true,
    "scope.WxFriendInteraction": true,
    "scope.gameClubData": true,
};

function _cloneAuthSetting() {
    const out = {};
    const keys = Object.keys(_authSetting);
    for (let i = 0; i < keys.length; i++) {
        out[keys[i]] = _authSetting[keys[i]];
    }
    return out;
}

// ---- getSetting ------------------------------------------------------------

function getSetting(options) {
    return wrapAsync('getSetting', function () {
        return { authSetting: _cloneAuthSetting() };
    }, options);
}

// ---- authorize -------------------------------------------------------------

function authorize(options) {
    return wrapAsync('authorize', function () {
        const opts = options || {};
        const scope = opts.scope;
        if (typeof scope !== 'string' || scope.length === 0) {
            throw new Error('scope is required');
        }
        _authSetting[scope] = true;
        return {};
    }, options);
}

// ---- openSetting (Mode C - host op) ---------------------------------------

const _openSettingApi = createDeferredApi('openSetting');

function openSetting(options) {
    return _openSettingApi.invoke(options, function (opts, requestId) {
        op_open_setting(JSON.stringify({ requestId: requestId }));
    });
}

function _internalOnOpenSettingResult(resultJson) {
    // Sync authSetting from host result before settling the promise
    var parsed;
    try { parsed = JSON.parse(resultJson); } catch (_) { parsed = {}; }
    if (parsed.authSetting && typeof parsed.authSetting === 'object') {
        var keys = Object.keys(parsed.authSetting);
        for (var i = 0; i < keys.length; i++) {
            _authSetting[keys[i]] = !!parsed.authSetting[keys[i]];
        }
    }
    _openSettingApi.settle(resultJson);
}

// ---- host-side helpers -----------------------------------------------------

// Called from Rust / EvalScript to update a specific scope's auth state.
//   _internalUpdateAuthSetting('scope.userLocation', false)
function _internalUpdateAuthSetting(scope, authorized) {
    if (typeof scope === 'string' && scope.length > 0) {
        _authSetting[scope] = !!authorized;
    }
}

// ---- Privacy APIs --------------------------------------------------------
// @stub getPrivacySetting returns hardcoded { needAuthorization: false }.
// @stub openPrivacyContract is a no-op.
// @stub onNeedPrivacyAuthorization listener infra is ready but host-side
//       dispatch (HostCommand or EvalScript) is not yet wired.

function getPrivacySetting(options) {
    return wrapAsync('getPrivacySetting', function () {
        return { needAuthorization: false, privacyContractName: '' };
    }, options);
}

function openPrivacyContract(options) {
    return wrapAsync('openPrivacyContract', function () {}, options);
}

// ---- onNeedPrivacyAuthorization / offNeedPrivacyAuthorization (Mode D) ---

var _privacyAuthListeners = [];

function onNeedPrivacyAuthorization(listener) {
    if (typeof listener === 'function') {
        _privacyAuthListeners.push(listener);
    }
}

function offNeedPrivacyAuthorization(listener) {
    if (typeof listener === 'function') {
        var index = _privacyAuthListeners.indexOf(listener);
        if (index !== -1) {
            _privacyAuthListeners.splice(index, 1);
        }
    } else {
        _privacyAuthListeners.length = 0;
    }
}

function _internalTriggerNeedPrivacyAuthorization(resolve) {
    for (var i = 0; i < _privacyAuthListeners.length; i++) {
        try { _privacyAuthListeners[i]({ resolve: resolve }); } catch (e) {
            console.error('onNeedPrivacyAuthorization listener error:', e);
        }
    }
}

// ---- requirePrivacyAuthorize (stub - resolves immediately) -----------------

function requirePrivacyAuthorize(options) {
    return wrapAsync('requirePrivacyAuthorize', function () {
        return {};
    }, options);
}

// ---- requestSubscribeMessage (stub - simulates all accepted) ---------------

function requestSubscribeMessage(options) {
    return wrapAsync('requestSubscribeMessage', function () {
        var opts = options || {};
        var tmplIds = opts.tmplIds || [];
        var result = {};
        for (var i = 0; i < tmplIds.length; i++) {
            result[tmplIds[i]] = 'accept';
        }
        return result;
    }, options);
}

export {
    getSetting,
    authorize,
    openSetting,
    _internalOnOpenSettingResult,
    _internalUpdateAuthSetting,
    getPrivacySetting,
    openPrivacyContract,
    onNeedPrivacyAuthorization,
    offNeedPrivacyAuthorization,
    _internalTriggerNeedPrivacyAuthorization,
    requirePrivacyAuthorize,
    requestSubscribeMessage,
};
