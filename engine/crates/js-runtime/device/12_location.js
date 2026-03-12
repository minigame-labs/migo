import { op_get_location, op_get_fuzzy_location } from "ext:core/ops";
import { createDeferredApi } from "ext:host_v8_base/02_async.js";

const _locationApi = createDeferredApi('getLocation');
const _fuzzyLocationApi = createDeferredApi('getFuzzyLocation');

function getLocation(options = {}) {
    return _locationApi.invoke(options, function (opts) {
        op_get_location(JSON.stringify({
            type: opts.type || 'wgs84',
            altitude: !!opts.altitude,
            isHighAccuracy: !!opts.isHighAccuracy,
            highAccuracyExpireTime: opts.highAccuracyExpireTime || 0,
        }));
    });
}

function _internalOnLocationResult(resultJson) {
    _locationApi.settle(resultJson);
}

function getFuzzyLocation(options = {}) {
    return _fuzzyLocationApi.invoke(options, function (opts) {
        op_get_fuzzy_location(JSON.stringify({
            type: opts.type || 'wgs84',
        }));
    });
}

function _internalOnFuzzyLocationResult(resultJson) {
    _fuzzyLocationApi.settle(resultJson);
}

export {
    getLocation,
    _internalOnLocationResult,
    getFuzzyLocation,
    _internalOnFuzzyLocationResult,
};
