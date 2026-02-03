import { op_get_battery_info } from "ext:core/ops";
import { wrapWxAsync } from "ext:host_v8_base/02_wx_async.js";

function getBatteryInfoSync() {
    try {
        return JSON.parse(op_get_battery_info());
    } catch (e) {
        return { level: "0", isCharging: false, isLowPowerModeEnabled: false };
    }
}

function getBatteryInfo(options) {
    return wrapWxAsync('getBatteryInfo', function () {
        return getBatteryInfoSync();
    }, options);
}

export { getBatteryInfo, getBatteryInfoSync };
