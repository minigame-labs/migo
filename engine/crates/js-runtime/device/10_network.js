import {
    op_start_network_monitoring,
    op_stop_network_monitoring,
    op_get_network_type,
    op_get_local_ip_address,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];

function onNetworkStatusChange(listener) {
    if (typeof listener === 'function') {
        if (_listeners.length === 0) {
            try {
                op_start_network_monitoring();
            } catch (e) {
                console.error('Failed to start network monitoring:', e);
            }
        }
        _listeners.push(listener);
    }
}

function offNetworkStatusChange(listener) {
    if (listener === undefined) {
        _listeners.length = 0;
    } else {
        const idx = _listeners.indexOf(listener);
        if (idx !== -1) {
            _listeners.splice(idx, 1);
        }
    }
    if (_listeners.length === 0) {
        try {
            op_stop_network_monitoring();
        } catch (e) {
            // Ignore stop errors
        }
    }
}

function _internalTriggerNetworkStatusChange(isConnected, networkType) {
    const data = { isConnected: isConnected, networkType: networkType };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onNetworkStatusChange listener error:', e);
        }
    }
}

function getNetworkType(options) {
    return wrapAsync('getNetworkType', function () {
        return JSON.parse(op_get_network_type());
    }, options);
}

function getLocalIPAddress(options) {
    return wrapAsync('getLocalIPAddress', function () {
        return JSON.parse(op_get_local_ip_address());
    }, options);
}

export {
    onNetworkStatusChange,
    offNetworkStatusChange,
    _internalTriggerNetworkStatusChange,
    getNetworkType,
    getLocalIPAddress,
};
