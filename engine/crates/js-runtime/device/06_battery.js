import { op_get_battery_info } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function getBatteryInfoSync() {
    try {
        return JSON.parse(op_get_battery_info());
    } catch (e) {
        return { level: "0", isCharging: false, isLowPowerModeEnabled: false };
    }
}

function getBatteryInfo(options) {
    return wrapAsync('getBatteryInfo', function () {
        return getBatteryInfoSync();
    }, options);
}

export { getBatteryInfo, getBatteryInfoSync };
