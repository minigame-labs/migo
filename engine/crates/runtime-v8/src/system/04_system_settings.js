
import { op_get_system_settings } from "ext:core/ops";

function getSystemSetting() {
    const settings = JSON.parse(op_get_system_settings());

    return {
        bluetoothEnabled: settings.bluetooth_enabled,
        locationEnabled: settings.location_enabled,
        wifiEnabled: settings.wifi_enabled,
        deviceOrientation: settings.orientation
    }
}

export { getSystemSetting }