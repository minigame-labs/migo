import {
    op_start_network_monitoring,
    op_stop_network_monitoring,
    op_get_network_type,
    op_get_local_ip_address,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const _listeners = [];
const _weakNetListeners = [];
let _isMonitoring = false;
let _lastWeakNet = false;

function _updateMonitoring() {
    const shouldMonitor = _listeners.length > 0 || _weakNetListeners.length > 0;
    if (shouldMonitor && !_isMonitoring) {
        try {
            op_start_network_monitoring();
            _isMonitoring = true;
            // Initialize _lastWeakNet
            try {
                const res = JSON.parse(op_get_network_type());
                _lastWeakNet = res.weakNet === true;
            } catch (e) {
                // Ignore
            }
        } catch (e) {
            console.error('Failed to start network monitoring:', e);
        }
    } else if (!shouldMonitor && _isMonitoring) {
        try {
            op_stop_network_monitoring();
            _isMonitoring = false;
        } catch (e) {
            // Ignore stop errors
        }
    }
}

function onNetworkStatusChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
        _updateMonitoring();
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
    _updateMonitoring();
}

function onNetworkWeakChange(listener) {
    if (typeof listener === 'function') {
        _weakNetListeners.push(listener);
        _updateMonitoring();
    }
}

function offNetworkWeakChange(listener) {
    if (listener === undefined) {
        _weakNetListeners.length = 0;
    } else {
        const idx = _weakNetListeners.indexOf(listener);
        if (idx !== -1) {
            _weakNetListeners.splice(idx, 1);
        }
    }
    _updateMonitoring();
}

function _internalTriggerNetworkStatusChange(isConnected, networkType) {
    // 1. Trigger standard network status change listeners
    if (_listeners.length > 0) {
        const data = { isConnected: isConnected, networkType: networkType };
        for (let i = 0; i < _listeners.length; i++) {
            try {
                _listeners[i](data);
            } catch (e) {
                console.error('onNetworkStatusChange listener error:', e);
            }
        }
    }

    // 2. Trigger weak network change listeners
    // We fetch the latest status to get weakNet field which is not passed in the event arguments
    if (_weakNetListeners.length > 0 || _isMonitoring) { 
        try {
            const res = JSON.parse(op_get_network_type());
            // Ensure we handle potential error response structure if op fails silently or returns error obj
            if (res && !res._error) {
                const currentWeakNet = res.weakNet === true;
                
                if (currentWeakNet !== _lastWeakNet) {
                    _lastWeakNet = currentWeakNet;
                    
                    if (_weakNetListeners.length > 0) {
                        const weakData = { 
                            weakNet: currentWeakNet, 
                            networkType: res.networkType 
                        };
                        for (let i = 0; i < _weakNetListeners.length; i++) {
                            try {
                                _weakNetListeners[i](weakData);
                            } catch (e) {
                                console.error('onNetworkWeakChange listener error:', e);
                            }
                        }
                    }
                }
            }
        } catch (e) {
            console.error('Error checking weak net status:', e);
        }
    }
}

function getNetworkType(options) {
    return wrapAsync('getNetworkType', function () {
        const res = JSON.parse(op_get_network_type());
        if (res && res._error) {
            let msg = res._error.errMsg || 'unknown error';
            msg = msg.replace(/^getNetworkType:fail:?/, '').trim();
            throw new Error(msg);
        }
        return res;
    }, options);
}

function getLocalIPAddress(options) {
    return wrapAsync('getLocalIPAddress', function () {
        const res = JSON.parse(op_get_local_ip_address());
        if (res && res._error) {
            let msg = res._error.errMsg || 'unknown error';
            msg = msg.replace(/^getLocalIPAddress:fail:?/, '').trim();
            throw new Error(msg);
        }
        return res;
    }, options);
}

export {
    onNetworkStatusChange,
    offNetworkStatusChange,
    onNetworkWeakChange,
    offNetworkWeakChange,
    _internalTriggerNetworkStatusChange,
    getNetworkType,
    getLocalIPAddress,
};
