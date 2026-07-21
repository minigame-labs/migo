import {
    op_show_toast,
    op_hide_toast,
    op_show_modal,
    op_show_loading,
    op_hide_loading,
    op_show_action_sheet,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

// ==================== Toast (Mode A) ====================

function showToast(options) {
    return wrapAsync('showToast', function () {
        var opts = options || {};
        op_show_toast(JSON.stringify({
            title: opts.title || '',
            icon: opts.icon || 'success',
            duration: opts.duration !== undefined ? opts.duration : 1500,
            mask: !!opts.mask,
        }));
    }, options);
}

function hideToast(options) {
    return wrapAsync('hideToast', function () {
        op_hide_toast();
    }, options);
}

// ==================== Modal (FIFO queue + Promise) ====================

var _pendingModals = [];

function showModal(options) {
    var opts = options || {};
    var success = typeof opts.success === 'function' ? opts.success : null;
    var fail = typeof opts.fail === 'function' ? opts.fail : null;
    var complete = typeof opts.complete === 'function' ? opts.complete : null;

    return new Promise(function (resolve, reject) {
        _pendingModals.push({ success: success, fail: fail, complete: complete, resolve: resolve, reject: reject });

        try {
            op_show_modal(JSON.stringify({
                title: opts.title || '',
                content: opts.content || '',
                showCancel: opts.showCancel !== false,
                cancelText: opts.cancelText || '\u53d6\u6d88',
                confirmText: opts.confirmText || '\u786e\u5b9a',
                cancelColor: opts.cancelColor || '#000000',
                confirmColor: opts.confirmColor || '#576B95',
            }));
        } catch (e) {
            _pendingModals.pop();
            var res = { errMsg: 'showModal:fail ' + e.message };
            if (fail) fail(res);
            if (complete) complete(res);
            reject(res);
        }
    });
}

function _internalOnModalResult(confirm, cancel) {
    var pending = _pendingModals.shift();
    if (!pending) return;

    var res = {
        confirm: !!confirm,
        cancel: !!cancel,
        errMsg: 'showModal:ok',
    };
    if (pending.success) pending.success(res);
    if (pending.complete) pending.complete(res);
    pending.resolve(res);
}

// ==================== Loading (Mode A) ====================

function showLoading(options) {
    return wrapAsync('showLoading', function () {
        var opts = options || {};
        op_show_loading(JSON.stringify({
            title: opts.title || '',
            mask: !!opts.mask,
        }));
    }, options);
}

function hideLoading(options) {
    return wrapAsync('hideLoading', function () {
        op_hide_loading();
    }, options);
}

// ==================== Action Sheet (FIFO queue + Promise) ====================

var _pendingActionSheets = [];

function showActionSheet(options) {
    var opts = options || {};
    var success = typeof opts.success === 'function' ? opts.success : null;
    var fail = typeof opts.fail === 'function' ? opts.fail : null;
    var complete = typeof opts.complete === 'function' ? opts.complete : null;

    return new Promise(function (resolve, reject) {
        _pendingActionSheets.push({ success: success, fail: fail, complete: complete, resolve: resolve, reject: reject });

        try {
            op_show_action_sheet(JSON.stringify({
                alertText: opts.alertText || '',
                itemList: opts.itemList || [],
                itemColor: opts.itemColor || '#000000',
            }));
        } catch (e) {
            _pendingActionSheets.pop();
            var res = { errMsg: 'showActionSheet:fail ' + e.message };
            if (fail) fail(res);
            if (complete) complete(res);
            reject(res);
        }
    });
}

function _internalOnActionSheetResult(tapIndex) {
    var pending = _pendingActionSheets.shift();
    if (!pending) return;

    if (tapIndex < 0) {
        var res = { errMsg: 'showActionSheet:fail cancel' };
        if (pending.fail) pending.fail(res);
        if (pending.complete) pending.complete(res);
        pending.reject(res);
    } else {
        var res = { tapIndex: tapIndex, errMsg: 'showActionSheet:ok' };
        if (pending.success) pending.success(res);
        if (pending.complete) pending.complete(res);
        pending.resolve(res);
    }
}

export {
    showToast, hideToast,
    showModal, _internalOnModalResult,
    showLoading, hideLoading,
    showActionSheet, _internalOnActionSheetResult,
};
