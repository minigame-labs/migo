// checkIsSupportMidasPayment / requestMidasPayment /
// requestMidasPaymentGameItem
//
// checkIsSupportMidasPayment: Mode B (sync, returns host JSON).
// requestMidasPayment/requestMidasPaymentGameItem: Mode C (async,
// host payment flow, result via EvalScript callback).
//
// Uses Map-based request tracking (same pattern as 13_login.js) to
// support concurrent payment requests without callback overwrite.

import {
    op_check_is_support_midas_payment,
    op_request_midas_payment,
    op_request_midas_payment_game_item,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const noop = function () {};

let _nextRequestId = 1;
const _pendingMidas = new Map();
const _pendingMidasGameItem = new Map();

function _nextId() {
    var id = _nextRequestId;
    _nextRequestId += 1;
    return id;
}

function _parseResult(resultJson) {
    if (typeof resultJson !== 'string' || resultJson.length === 0) return {};
    try {
        var p = JSON.parse(resultJson);
        return (p && typeof p === 'object') ? p : {};
    } catch (_) {
        return {};
    }
}

// ---- checkIsSupportMidasPayment ---------------------------------------------

function checkIsSupportMidasPayment(options) {
    return wrapAsync('checkIsSupportMidasPayment', function () {
        var resultJson = op_check_is_support_midas_payment(JSON.stringify({}));
        return JSON.parse(resultJson);
    }, options);
}

// ---- requestMidasPayment ---------------------------------------------------

function requestMidasPayment(options) {
    var opts = (options && typeof options === 'object') ? options : {};
    var requestId = _nextId();
    var success = typeof opts.success === 'function' ? opts.success : noop;
    var fail = typeof opts.fail === 'function' ? opts.fail : noop;
    var complete = typeof opts.complete === 'function' ? opts.complete : noop;

    return new Promise(function (resolve, reject) {
        _pendingMidas.set(requestId, { success: success, fail: fail, complete: complete, resolve: resolve, reject: reject });

        try {
            op_request_midas_payment(JSON.stringify({
                requestId: requestId,
                mode: opts.mode || 'game',
                env: opts.env !== undefined ? opts.env : 0,
                offerId: opts.offerId || '',
                currencyType: opts.currencyType || 'CNY',
                platform: opts.platform || '',
                buyQuantity: opts.buyQuantity || 0,
                zoneId: opts.zoneId || 1,
                outTradeNo: opts.outTradeNo || '',
            }));
        } catch (e) {
            _pendingMidas.delete(requestId);
            var res = { errMsg: 'requestMidasPayment:fail ' + (e.message || e), errCode: -1 };
            queueMicrotask(function () { fail(res); complete(res); reject(res); });
        }
    });
}

function _settleMidas(requestId, result) {
    var pending = _pendingMidas.get(requestId);
    if (!pending) return;
    _pendingMidas.delete(requestId);

    if (result.error) {
        var res = { errMsg: result.error };
        if (result.errCode !== undefined) res.errCode = result.errCode;
        pending.fail(res);
        pending.complete(res);
        pending.reject(res);
    } else {
        var res = { errMsg: 'requestMidasPayment:ok' };
        pending.success(res);
        pending.complete(res);
        pending.resolve(res);
    }
}

function _internalOnMidasPaymentResult(resultJson) {
    var result = _parseResult(resultJson);
    var requestId = Number(result.requestId);
    if (Number.isFinite(requestId)) {
        _settleMidas(requestId, result);
    } else {
        // Fallback: settle the oldest pending request (single-flight compat)
        var keys = _pendingMidas.keys();
        var first = keys.next();
        if (!first.done) {
            _settleMidas(first.value, result);
        }
    }
}

// ---- requestMidasPaymentGameItem -------------------------------------------

function requestMidasPaymentGameItem(options) {
    var opts = (options && typeof options === 'object') ? options : {};
    var requestId = _nextId();
    var success = typeof opts.success === 'function' ? opts.success : noop;
    var fail = typeof opts.fail === 'function' ? opts.fail : noop;
    var complete = typeof opts.complete === 'function' ? opts.complete : noop;

    return new Promise(function (resolve, reject) {
        _pendingMidasGameItem.set(requestId, { success: success, fail: fail, complete: complete, resolve: resolve, reject: reject });

        try {
            op_request_midas_payment_game_item(JSON.stringify({
                requestId: requestId,
                signData: opts.signData || '',
                paySig: opts.paySig || '',
                signature: opts.signature || '',
            }));
        } catch (e) {
            _pendingMidasGameItem.delete(requestId);
            var res = { errMsg: 'requestMidasPaymentGameItem:fail ' + (e.message || e), errCode: -1 };
            queueMicrotask(function () { fail(res); complete(res); reject(res); });
        }
    });
}

function _settleMidasGameItem(requestId, result) {
    var pending = _pendingMidasGameItem.get(requestId);
    if (!pending) return;
    _pendingMidasGameItem.delete(requestId);

    if (result.error) {
        var res = { errMsg: result.error };
        if (result.errCode !== undefined) res.errCode = result.errCode;
        pending.fail(res);
        pending.complete(res);
        pending.reject(res);
    } else {
        var res = { errMsg: 'requestMidasPaymentGameItem:ok' };
        pending.success(res);
        pending.complete(res);
        pending.resolve(res);
    }
}

function _internalOnMidasPaymentGameItemResult(resultJson) {
    var result = _parseResult(resultJson);
    var requestId = Number(result.requestId);
    if (Number.isFinite(requestId)) {
        _settleMidasGameItem(requestId, result);
    } else {
        // Fallback: settle the oldest pending request (single-flight compat)
        var keys = _pendingMidasGameItem.keys();
        var first = keys.next();
        if (!first.done) {
            _settleMidasGameItem(first.value, result);
        }
    }
}

export {
    checkIsSupportMidasPayment,
    requestMidasPayment,
    requestMidasPaymentGameItem,
    _internalOnMidasPaymentResult,
    _internalOnMidasPaymentGameItemResult,
};
