import { op_open_app_authorize_setting } from "ext:core/ops";

var _pendingAuthSetting = [];

function openAppAuthorizeSetting(options) {
    var opts = options || {};
    var success = typeof opts.success === 'function' ? opts.success : null;
    var fail = typeof opts.fail === 'function' ? opts.fail : null;
    var complete = typeof opts.complete === 'function' ? opts.complete : null;

    return new Promise(function (resolve, reject) {
        _pendingAuthSetting.push({ success: success, fail: fail, complete: complete, resolve: resolve, reject: reject });

        try {
            op_open_app_authorize_setting();
        } catch (e) {
            _pendingAuthSetting.pop();
            var res = { errMsg: 'openAppAuthorizeSetting:fail ' + e.message };
            if (fail) fail(res);
            if (complete) complete(res);
            reject(res);
        }
    });
}

function _internalOnOpenAppAuthorizeSettingFinished(code) {
    var pending = _pendingAuthSetting.shift();
    if (!pending) return;

    if (code >= 0) {
        var res = { errMsg: 'openAppAuthorizeSetting:ok' };
        if (pending.success) pending.success(res);
        if (pending.complete) pending.complete(res);
        pending.resolve(res);
    } else {
        var res = { errMsg: 'openAppAuthorizeSetting:fail' };
        if (pending.fail) pending.fail(res);
        if (pending.complete) pending.complete(res);
        pending.reject(res);
    }
}

export { openAppAuthorizeSetting, _internalOnOpenAppAuthorizeSettingFinished };
