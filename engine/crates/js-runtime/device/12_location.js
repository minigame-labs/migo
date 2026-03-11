import { op_get_location, op_get_fuzzy_location } from "ext:core/ops";

const noop = () => {};

// ==================== getLocation (async callback) ====================

let _getLocationSuccess = null;
let _getLocationFail = null;
let _getLocationComplete = null;

function getLocation(options = {}) {
    const { type, altitude, isHighAccuracy, highAccuracyExpireTime, success, fail, complete } = options;

    _getLocationSuccess = success || noop;
    _getLocationFail = fail || noop;
    _getLocationComplete = complete || noop;

    try {
        op_get_location(JSON.stringify({
            type: type || 'wgs84',
            altitude: !!altitude,
            isHighAccuracy: !!isHighAccuracy,
            highAccuracyExpireTime: highAccuracyExpireTime || 0,
        }));
    } catch (e) {
        const res = { errMsg: 'getLocation:fail ' + e.message };
        _getLocationFail(res);
        _getLocationComplete(res);
        _getLocationSuccess = null;
        _getLocationFail = null;
        _getLocationComplete = null;
    }
}

function _internalOnLocationResult(resultJson) {
    let parsed;
    try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

    if (parsed.error) {
        const res = { errMsg: 'getLocation:fail ' + parsed.error };
        const f = _getLocationFail;
        const c = _getLocationComplete;
        _getLocationSuccess = null;
        _getLocationFail = null;
        _getLocationComplete = null;
        if (f) f(res);
        if (c) c(res);
    } else {
        const res = { errMsg: 'getLocation:ok', ...parsed };
        const s = _getLocationSuccess;
        const c = _getLocationComplete;
        _getLocationSuccess = null;
        _getLocationFail = null;
        _getLocationComplete = null;
        if (s) s(res);
        if (c) c(res);
    }
}

// ==================== getFuzzyLocation (async callback) ====================

let _getFuzzyLocationSuccess = null;
let _getFuzzyLocationFail = null;
let _getFuzzyLocationComplete = null;

function getFuzzyLocation(options = {}) {
    const { type, success, fail, complete } = options;

    _getFuzzyLocationSuccess = success || noop;
    _getFuzzyLocationFail = fail || noop;
    _getFuzzyLocationComplete = complete || noop;

    try {
        op_get_fuzzy_location(JSON.stringify({
            type: type || 'wgs84',
        }));
    } catch (e) {
        const res = { errMsg: 'getFuzzyLocation:fail ' + e.message };
        _getFuzzyLocationFail(res);
        _getFuzzyLocationComplete(res);
        _getFuzzyLocationSuccess = null;
        _getFuzzyLocationFail = null;
        _getFuzzyLocationComplete = null;
    }
}

function _internalOnFuzzyLocationResult(resultJson) {
    let parsed;
    try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

    if (parsed.error) {
        const res = { errMsg: 'getFuzzyLocation:fail ' + parsed.error };
        const f = _getFuzzyLocationFail;
        const c = _getFuzzyLocationComplete;
        _getFuzzyLocationSuccess = null;
        _getFuzzyLocationFail = null;
        _getFuzzyLocationComplete = null;
        if (f) f(res);
        if (c) c(res);
    } else {
        const res = { errMsg: 'getFuzzyLocation:ok', ...parsed };
        const s = _getFuzzyLocationSuccess;
        const c = _getFuzzyLocationComplete;
        _getFuzzyLocationSuccess = null;
        _getFuzzyLocationFail = null;
        _getFuzzyLocationComplete = null;
        if (s) s(res);
        if (c) c(res);
    }
}

export {
    getLocation,
    _internalOnLocationResult,
    getFuzzyLocation,
    _internalOnFuzzyLocationResult,
};
