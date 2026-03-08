import {
    op_open_system_bluetooth_setting,
    op_open_bluetooth_adapter,
    op_close_bluetooth_adapter,
    op_get_bluetooth_adapter_state,
    op_start_bluetooth_devices_discovery,
    op_stop_bluetooth_devices_discovery,
    op_get_bluetooth_devices,
    op_get_connected_bluetooth_devices,
    op_make_bluetooth_pair,
    op_is_bluetooth_device_paired,
    op_start_beacon_discovery,
    op_stop_beacon_discovery,
    op_get_beacons,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

// ==================== System Bluetooth Setting ====================

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

// ==================== Bluetooth Adapter APIs ====================

function openBluetoothAdapter(options = {}) {
    const { mode = 'central' } = options;
    return wrapAsync('openBluetoothAdapter', function () {
        op_open_bluetooth_adapter(JSON.stringify({ mode }));
    }, options);
}

function closeBluetoothAdapter(options = {}) {
    return wrapAsync('closeBluetoothAdapter', function () {
        op_close_bluetooth_adapter();
    }, options);
}

function getBluetoothAdapterState(options = {}) {
    return wrapAsync('getBluetoothAdapterState', function () {
        const json = op_get_bluetooth_adapter_state();
        return JSON.parse(json);
    }, options);
}

// ==================== Device Discovery APIs ====================

function startBluetoothDevicesDiscovery(options = {}) {
    const { services, allowDuplicatesKey = false, interval = 0, powerLevel = 'medium' } = options;
    return wrapAsync('startBluetoothDevicesDiscovery', function () {
        op_start_bluetooth_devices_discovery(JSON.stringify({
            services: services || [],
            allowDuplicatesKey,
            interval,
            powerLevel,
        }));
    }, options);
}

function stopBluetoothDevicesDiscovery(options = {}) {
    return wrapAsync('stopBluetoothDevicesDiscovery', function () {
        op_stop_bluetooth_devices_discovery();
    }, options);
}

function getBluetoothDevices(options = {}) {
    return wrapAsync('getBluetoothDevices', function () {
        const json = op_get_bluetooth_devices();
        return JSON.parse(json);
    }, options);
}

function getConnectedBluetoothDevices(options = {}) {
    const { services } = options;
    return wrapAsync('getConnectedBluetoothDevices', function () {
        const json = op_get_connected_bluetooth_devices(JSON.stringify({
            services: services || [],
        }));
        return JSON.parse(json);
    }, options);
}

// ==================== Pairing APIs ====================

function makeBluetoothPair(options = {}) {
    const { deviceId, pin, timeout = 20000 } = options;
    return wrapAsync('makeBluetoothPair', function () {
        op_make_bluetooth_pair(JSON.stringify({ deviceId, pin, timeout }));
    }, options);
}

function isBluetoothDevicePaired(options = {}) {
    const { deviceId } = options;
    return wrapAsync('isBluetoothDevicePaired', function () {
        op_is_bluetooth_device_paired(JSON.stringify({ deviceId }));
    }, options);
}

// ==================== Bluetooth Event Listeners ====================

const _adapterStateChangeListeners = [];
const _deviceFoundListeners = [];

function onBluetoothAdapterStateChange(listener) {
    if (typeof listener === 'function') {
        _adapterStateChangeListeners.push(listener);
    }
}

function offBluetoothAdapterStateChange(listener) {
    if (typeof listener === 'function') {
        const index = _adapterStateChangeListeners.indexOf(listener);
        if (index !== -1) {
            _adapterStateChangeListeners.splice(index, 1);
        }
    } else {
        _adapterStateChangeListeners.length = 0;
    }
}

function onBluetoothDeviceFound(listener) {
    if (typeof listener === 'function') {
        _deviceFoundListeners.push(listener);
    }
}

function offBluetoothDeviceFound(listener) {
    if (typeof listener === 'function') {
        const index = _deviceFoundListeners.indexOf(listener);
        if (index !== -1) {
            _deviceFoundListeners.splice(index, 1);
        }
    } else {
        _deviceFoundListeners.length = 0;
    }
}

// ==================== Internal Trigger Functions ====================

function _internalTriggerBluetoothAdapterStateChange(available, discovering) {
    const data = { available, discovering };
    for (let i = 0; i < _adapterStateChangeListeners.length; i++) {
        try { _adapterStateChangeListeners[i](data); } catch (e) {
            console.error('onBluetoothAdapterStateChange listener error:', e);
        }
    }
}

function _internalTriggerBluetoothDeviceFound(devicesJson) {
    const data = { devices: JSON.parse(devicesJson) };
    for (let i = 0; i < _deviceFoundListeners.length; i++) {
        try { _deviceFoundListeners[i](data); } catch (e) {
            console.error('onBluetoothDeviceFound listener error:', e);
        }
    }
}

// ==================== Beacon APIs ====================

function startBeaconDiscovery(options = {}) {
    const { uuids, ignoreBluetoothAvailable = false } = options;
    return wrapAsync('startBeaconDiscovery', function () {
        op_start_beacon_discovery(JSON.stringify({
            uuids: uuids || [],
            ignoreBluetoothAvailable,
        }));
    }, options);
}

function stopBeaconDiscovery(options = {}) {
    return wrapAsync('stopBeaconDiscovery', function () {
        op_stop_beacon_discovery();
    }, options);
}

function getBeacons(options = {}) {
    return wrapAsync('getBeacons', function () {
        const json = op_get_beacons();
        return JSON.parse(json);
    }, options);
}

// ==================== Beacon Event Listeners ====================

const _beaconUpdateListeners = [];
const _beaconServiceChangeListeners = [];

function onBeaconUpdate(listener) {
    if (typeof listener === 'function') {
        _beaconUpdateListeners.push(listener);
    }
}

function offBeaconUpdate(listener) {
    if (typeof listener === 'function') {
        const index = _beaconUpdateListeners.indexOf(listener);
        if (index !== -1) {
            _beaconUpdateListeners.splice(index, 1);
        }
    } else {
        _beaconUpdateListeners.length = 0;
    }
}

function onBeaconServiceChange(listener) {
    if (typeof listener === 'function') {
        _beaconServiceChangeListeners.push(listener);
    }
}

function offBeaconServiceChange(listener) {
    if (typeof listener === 'function') {
        const index = _beaconServiceChangeListeners.indexOf(listener);
        if (index !== -1) {
            _beaconServiceChangeListeners.splice(index, 1);
        }
    } else {
        _beaconServiceChangeListeners.length = 0;
    }
}

// ==================== Beacon Internal Trigger Functions ====================

function _internalTriggerBeaconUpdate(beaconsJson) {
    const data = { beacons: JSON.parse(beaconsJson) };
    for (let i = 0; i < _beaconUpdateListeners.length; i++) {
        try { _beaconUpdateListeners[i](data); } catch (e) {
            console.error('onBeaconUpdate listener error:', e);
        }
    }
}

function _internalTriggerBeaconServiceChange(available, discovering) {
    const data = { available, discovering };
    for (let i = 0; i < _beaconServiceChangeListeners.length; i++) {
        try { _beaconServiceChangeListeners[i](data); } catch (e) {
            console.error('onBeaconServiceChange listener error:', e);
        }
    }
}

export {
    // System bluetooth setting
    openSystemBluetoothSetting,
    _internalOnOpenBluetoothSettingFinished,
    // Bluetooth adapter
    openBluetoothAdapter,
    closeBluetoothAdapter,
    getBluetoothAdapterState,
    // Device discovery
    startBluetoothDevicesDiscovery,
    stopBluetoothDevicesDiscovery,
    getBluetoothDevices,
    getConnectedBluetoothDevices,
    // Pairing
    makeBluetoothPair,
    isBluetoothDevicePaired,
    // Bluetooth events
    onBluetoothAdapterStateChange,
    offBluetoothAdapterStateChange,
    onBluetoothDeviceFound,
    offBluetoothDeviceFound,
    _internalTriggerBluetoothAdapterStateChange,
    _internalTriggerBluetoothDeviceFound,
    // Beacon
    startBeaconDiscovery,
    stopBeaconDiscovery,
    getBeacons,
    onBeaconUpdate,
    offBeaconUpdate,
    onBeaconServiceChange,
    offBeaconServiceChange,
    _internalTriggerBeaconUpdate,
    _internalTriggerBeaconServiceChange,
};
