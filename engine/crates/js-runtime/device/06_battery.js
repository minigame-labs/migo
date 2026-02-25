import { op_get_battery_info } from "ext:core/ops";
import { promisify } from "ext:host_v8_base/02_async.js";

function getBatteryInfoSync() {
    try {
        return JSON.parse(op_get_battery_info());
    } catch (e) {
        return { level: "0", isCharging: false, isLowPowerModeEnabled: false };
    }
}

const getBatteryInfo = promisify('getBatteryInfo', () => getBatteryInfoSync());

export { getBatteryInfo, getBatteryInfoSync };
