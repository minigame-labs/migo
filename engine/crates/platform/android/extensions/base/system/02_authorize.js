import { op_open_app_authorize_setting } from "ext:core/ops";

const noop = () => { };

let onOpenAppAuthorizeSettingSuccess = noop;
let onOpenAppAuthorizeSettingFail = noop;
let onOpenAppAuthorizeSettingComplete = noop;

function openAppAuthorizeSetting({ success, fail, complete } = {}) {
    onOpenAppAuthorizeSettingSuccess = success || noop;
    onOpenAppAuthorizeSettingFail = fail || noop;
    onOpenAppAuthorizeSettingComplete = complete || noop;

    op_open_app_authorize_setting();
}

function _internalOnOpenAppAuthorizeSettingFinished(code) {
    if (code >= 0) {
        onOpenAppAuthorizeSettingSuccess({ "code": code, "message": "App authorization settings opened successfully" });
    } else {
        onOpenAppAuthorizeSettingFail({ "code": code, "message": "Failed to open app authorization settings" });
    }
    onOpenAppAuthorizeSettingComplete({ "code": code });

    onOpenAppAuthorizeSettingSuccess = noop;
    onOpenAppAuthorizeSettingFail = noop;
    onOpenAppAuthorizeSettingComplete = noop;
}

export { openAppAuthorizeSetting, _internalOnOpenAppAuthorizeSettingFinished }
