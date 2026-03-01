import { op_open_system_bluetooth_setting } from "ext:core/ops";

const noop = () => { };

let onOpenBluetoothSettingSuccess = noop;
let onOpenBluetoothSettingFail = noop;
let onOpenBluetoothSettingComplete = noop;

function openSystemBluetoothSetting({ success, fail, complete } = {}) {
    onOpenBluetoothSettingSuccess = success || noop;
    onOpenBluetoothSettingFail = fail || noop;
    onOpenBluetoothSettingComplete = complete || noop;

    op_open_system_bluetooth_setting();
}

// TODO: real code
function _internalOnOpenBluetoothSettingFinished(code) {
    if (code >= 0) {
        onOpenBluetoothSettingSuccess({ "code": code, "message": "Bluetooth settings opened successfully" });
    } else {
        onOpenBluetoothSettingFail({ "code": code, "message": "Failed to open Bluetooth settings" });
    }
    onOpenBluetoothSettingComplete({ "code": code });

    onOpenBluetoothSettingSuccess = noop;
    onOpenBluetoothSettingFail = noop;
    onOpenBluetoothSettingComplete = noop;
}

export { openSystemBluetoothSetting, _internalOnOpenBluetoothSettingFinished }