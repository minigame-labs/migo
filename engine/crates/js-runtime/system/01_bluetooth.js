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
    op_create_ble_connection,
    op_close_ble_connection,
    op_get_ble_device_services,
    op_get_ble_device_characteristics,
    op_read_ble_characteristic_value,
    op_write_ble_characteristic_value,
    op_notify_ble_characteristic_value_change,
    op_get_ble_device_rssi,
    op_set_ble_mtu,
    op_get_ble_mtu,
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

var _discoveryTimer = null;

function startBluetoothDevicesDiscovery(options = {}) {
    const { services, allowDuplicatesKey = false, interval = 0, powerLevel = 'medium' } = options;
    return wrapAsync('startBluetoothDevicesDiscovery', function () {
        op_start_bluetooth_devices_discovery(JSON.stringify({
            services: services || [],
            allowDuplicatesKey,
            interval,
            powerLevel,
        }));
        // Auto-stop scanning after 30 seconds to prevent battery drain.
        if (_discoveryTimer !== null) {
            clearTimeout(_discoveryTimer);
        }
        _discoveryTimer = setTimeout(function () {
            _discoveryTimer = null;
            try { op_stop_bluetooth_devices_discovery(); } catch (_) {}
        }, 30000);
    }, options);
}

function stopBluetoothDevicesDiscovery(options = {}) {
    return wrapAsync('stopBluetoothDevicesDiscovery', function () {
        if (_discoveryTimer !== null) {
            clearTimeout(_discoveryTimer);
            _discoveryTimer = null;
        }
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

// ==================== BLE GATT Connection APIs ====================

function createBLEConnection(options = {}) {
    const { deviceId, timeout } = options;
    return wrapAsync('createBLEConnection', function () {
        op_create_ble_connection(JSON.stringify({
            deviceId: deviceId || '',
            timeout: timeout !== undefined ? timeout : 0,
        }));
    }, options);
}

function closeBLEConnection(options = {}) {
    const { deviceId } = options;
    return wrapAsync('closeBLEConnection', function () {
        op_close_ble_connection(JSON.stringify({
            deviceId: deviceId || '',
        }));
    }, options);
}

// ==================== BLE GATT Service/Characteristic APIs ====================

function getBLEDeviceServices(options = {}) {
    const { deviceId } = options;
    return wrapAsync('getBLEDeviceServices', function () {
        const json = op_get_ble_device_services(JSON.stringify({
            deviceId: deviceId || '',
        }));
        return JSON.parse(json);
    }, options);
}

function getBLEDeviceCharacteristics(options = {}) {
    const { deviceId, serviceId } = options;
    return wrapAsync('getBLEDeviceCharacteristics', function () {
        const json = op_get_ble_device_characteristics(JSON.stringify({
            deviceId: deviceId || '',
            serviceId: serviceId || '',
        }));
        return JSON.parse(json);
    }, options);
}

function readBLECharacteristicValue(options = {}) {
    const { deviceId, serviceId, characteristicId } = options;
    return wrapAsync('readBLECharacteristicValue', function () {
        op_read_ble_characteristic_value(JSON.stringify({
            deviceId: deviceId || '',
            serviceId: serviceId || '',
            characteristicId: characteristicId || '',
        }));
    }, options);
}

function _bufferToHex(buf) {
    if (!buf) return '';
    var bytes;
    if (buf instanceof ArrayBuffer) {
        bytes = new Uint8Array(buf);
    } else if (ArrayBuffer.isView(buf)) {
        bytes = new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    } else if (typeof buf === 'string') {
        return buf; // already hex string
    } else {
        return '';
    }
    var hex = '';
    for (var i = 0; i < bytes.length; i++) {
        var b = bytes[i].toString(16);
        hex += b.length < 2 ? '0' + b : b;
    }
    return hex;
}

function writeBLECharacteristicValue(options = {}) {
    const { deviceId, serviceId, characteristicId, value, writeType } = options;
    return wrapAsync('writeBLECharacteristicValue', function () {
        op_write_ble_characteristic_value(JSON.stringify({
            deviceId: deviceId || '',
            serviceId: serviceId || '',
            characteristicId: characteristicId || '',
            value: _bufferToHex(value),
            writeType: writeType || 'write',
        }));
    }, options);
}

function notifyBLECharacteristicValueChange(options = {}) {
    const { deviceId, serviceId, characteristicId, state } = options;
    return wrapAsync('notifyBLECharacteristicValueChange', function () {
        op_notify_ble_characteristic_value_change(JSON.stringify({
            deviceId: deviceId || '',
            serviceId: serviceId || '',
            characteristicId: characteristicId || '',
            state: state !== undefined ? state : false,
        }));
    }, options);
}

// ==================== BLE RSSI / MTU APIs ====================

function getBLEDeviceRSSI(options = {}) {
    const { deviceId } = options;
    return wrapAsync('getBLEDeviceRSSI', function () {
        const json = op_get_ble_device_rssi(JSON.stringify({
            deviceId: deviceId || '',
        }));
        return JSON.parse(json);
    }, options);
}

function setBLEMTU(options = {}) {
    const { deviceId, mtu } = options;
    return wrapAsync('setBLEMTU', function () {
        op_set_ble_mtu(JSON.stringify({
            deviceId: deviceId || '',
            mtu: mtu !== undefined ? mtu : 23,
        }));
    }, options);
}

function getBLEMTU(options = {}) {
    const { deviceId } = options;
    return wrapAsync('getBLEMTU', function () {
        const json = op_get_ble_mtu(JSON.stringify({
            deviceId: deviceId || '',
        }));
        return JSON.parse(json);
    }, options);
}

// ==================== BLE GATT Event Listeners ====================

const _bleConnectionStateChangeListeners = [];
const _bleCharacteristicValueChangeListeners = [];
const _bleMTUChangeListeners = [];

function onBLEConnectionStateChange(listener) {
    if (typeof listener === 'function') {
        _bleConnectionStateChangeListeners.push(listener);
    }
}

function offBLEConnectionStateChange(listener) {
    if (typeof listener === 'function') {
        const index = _bleConnectionStateChangeListeners.indexOf(listener);
        if (index !== -1) {
            _bleConnectionStateChangeListeners.splice(index, 1);
        }
    } else {
        _bleConnectionStateChangeListeners.length = 0;
    }
}

function onBLECharacteristicValueChange(listener) {
    if (typeof listener === 'function') {
        _bleCharacteristicValueChangeListeners.push(listener);
    }
}

function offBLECharacteristicValueChange(listener) {
    if (typeof listener === 'function') {
        const index = _bleCharacteristicValueChangeListeners.indexOf(listener);
        if (index !== -1) {
            _bleCharacteristicValueChangeListeners.splice(index, 1);
        }
    } else {
        _bleCharacteristicValueChangeListeners.length = 0;
    }
}

function onBLEMTUChange(listener) {
    if (typeof listener === 'function') {
        _bleMTUChangeListeners.push(listener);
    }
}

function offBLEMTUChange(listener) {
    if (typeof listener === 'function') {
        const index = _bleMTUChangeListeners.indexOf(listener);
        if (index !== -1) {
            _bleMTUChangeListeners.splice(index, 1);
        }
    } else {
        _bleMTUChangeListeners.length = 0;
    }
}

// ==================== BLE GATT Internal Trigger Functions ====================

function _internalTriggerBLEConnectionStateChange(deviceId, connected) {
    const data = { deviceId: deviceId, connected: connected };
    for (let i = 0; i < _bleConnectionStateChangeListeners.length; i++) {
        try { _bleConnectionStateChangeListeners[i](data); } catch (e) {
            console.error('onBLEConnectionStateChange listener error:', e);
        }
    }
}

function _internalTriggerBLECharacteristicValueChange(deviceId, serviceId, characteristicId, value) {
    const data = {
        deviceId: deviceId,
        serviceId: serviceId,
        characteristicId: characteristicId,
        value: value,
    };
    for (let i = 0; i < _bleCharacteristicValueChangeListeners.length; i++) {
        try { _bleCharacteristicValueChangeListeners[i](data); } catch (e) {
            console.error('onBLECharacteristicValueChange listener error:', e);
        }
    }
}

function _internalTriggerBLEMTUChange(deviceId, mtu) {
    const data = { deviceId: deviceId, mtu: mtu };
    for (let i = 0; i < _bleMTUChangeListeners.length; i++) {
        try { _bleMTUChangeListeners[i](data); } catch (e) {
            console.error('onBLEMTUChange listener error:', e);
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
    // BLE GATT
    createBLEConnection,
    closeBLEConnection,
    getBLEDeviceServices,
    getBLEDeviceCharacteristics,
    readBLECharacteristicValue,
    writeBLECharacteristicValue,
    notifyBLECharacteristicValueChange,
    getBLEDeviceRSSI,
    setBLEMTU,
    getBLEMTU,
    // BLE GATT events
    onBLEConnectionStateChange,
    offBLEConnectionStateChange,
    onBLECharacteristicValueChange,
    offBLECharacteristicValueChange,
    onBLEMTUChange,
    offBLEMTUChange,
    _internalTriggerBLEConnectionStateChange,
    _internalTriggerBLECharacteristicValueChange,
    _internalTriggerBLEMTUChange,
};
