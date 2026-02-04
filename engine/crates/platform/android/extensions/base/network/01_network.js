import {
    op_start_network_monitoring,
    op_stop_network_monitoring,
    op_get_network_type,
    op_get_local_ip_address,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

// Internal: Start monitoring (called when first listener is registered)
function startNetworkMonitoring() {
    op_start_network_monitoring();
}

// Internal: Stop monitoring (called when all listeners are removed)
function stopNetworkMonitoring() {
    op_stop_network_monitoring();
}

function getNetworkType(options) {
    return wrapAsync('getNetworkType', function () {
        const json = op_get_network_type();
        return JSON.parse(json);
    }, options);
}

function getLocalIPAddress(options) {
    return wrapAsync('getLocalIPAddress', function () {
        const json = op_get_local_ip_address();
        const result = JSON.parse(json);
        result.errMsg = 'getLocalIPAddress:ok';
        return result;
    }, options);
}

export {
    startNetworkMonitoring,
    stopNetworkMonitoring,
    getNetworkType,
    getLocalIPAddress,
};
