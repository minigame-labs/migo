import {
    op_show_toast,
    op_hide_toast,
    op_show_modal,
    op_show_loading,
    op_hide_loading,
} from "ext:core/ops";

const noop = () => {};

// ==================== Toast ====================

function showToast({ title = '', icon = 'success', duration = 1500, mask = false,
                     success, fail, complete } = {}) {
    try {
        op_show_toast(JSON.stringify({ title, icon, duration, mask }));
        const res = { errMsg: 'showToast:ok' };
        (success || noop)(res);
        (complete || noop)(res);
    } catch (e) {
        const res = { errMsg: 'showToast:fail ' + e.message };
        (fail || noop)(res);
        (complete || noop)(res);
    }
}

function hideToast({ success, fail, complete } = {}) {
    try {
        op_hide_toast();
        const res = { errMsg: 'hideToast:ok' };
        (success || noop)(res);
        (complete || noop)(res);
    } catch (e) {
        const res = { errMsg: 'hideToast:fail ' + e.message };
        (fail || noop)(res);
        (complete || noop)(res);
    }
}

// ==================== Modal ====================

let _modalSuccess = null;
let _modalFail = null;
let _modalComplete = null;

function showModal({ title = '', content = '', showCancel = true,
                     cancelText = '\u53d6\u6d88', confirmText = '\u786e\u5b9a',
                     cancelColor = '#000000', confirmColor = '#576B95',
                     success, fail, complete } = {}) {
    _modalSuccess = success || noop;
    _modalFail = fail || noop;
    _modalComplete = complete || noop;

    try {
        op_show_modal(JSON.stringify({
            title, content, showCancel, cancelText, confirmText,
            cancelColor, confirmColor,
        }));
    } catch (e) {
        const res = { errMsg: 'showModal:fail ' + e.message };
        _modalFail(res);
        _modalComplete(res);
        _modalSuccess = null;
        _modalFail = null;
        _modalComplete = null;
    }
}

function _internalOnModalResult(confirm, cancel) {
    const res = {
        confirm: !!confirm,
        cancel: !!cancel,
        errMsg: 'showModal:ok',
    };

    const s = _modalSuccess;
    const c = _modalComplete;
    _modalSuccess = null;
    _modalFail = null;
    _modalComplete = null;

    if (s) s(res);
    if (c) c(res);
}

// ==================== Loading ====================

function showLoading({ title = '', mask = false, success, fail, complete } = {}) {
    try {
        op_show_loading(JSON.stringify({ title, mask }));
        const res = { errMsg: 'showLoading:ok' };
        (success || noop)(res);
        (complete || noop)(res);
    } catch (e) {
        const res = { errMsg: 'showLoading:fail ' + e.message };
        (fail || noop)(res);
        (complete || noop)(res);
    }
}

function hideLoading({ success, fail, complete } = {}) {
    try {
        op_hide_loading();
        const res = { errMsg: 'hideLoading:ok' };
        (success || noop)(res);
        (complete || noop)(res);
    } catch (e) {
        const res = { errMsg: 'hideLoading:fail ' + e.message };
        (fail || noop)(res);
        (complete || noop)(res);
    }
}

export {
    showToast, hideToast,
    showModal, _internalOnModalResult,
    showLoading, hideLoading,
};
