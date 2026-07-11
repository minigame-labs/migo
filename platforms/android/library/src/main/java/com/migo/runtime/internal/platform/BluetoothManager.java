package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.BluetoothGattDescriptor;
import android.bluetooth.BluetoothGattService;
import android.bluetooth.BluetoothProfile;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanFilter;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.Build;
import android.os.ParcelUuid;
import android.util.Log;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.lang.ref.WeakReference;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Manages Bluetooth adapter, BLE device discovery, pairing, and Beacon operations.
 * One instance per session.
 */
public class BluetoothManager {

    private static final String TAG = "BluetoothManager";

    private final int sessionId;
    private final WeakReference<Activity> activityRef;
    private final BluetoothAdapter adapter;

    private boolean adapterOpened = false;
    private volatile boolean discovering = false;

    /** Discovered devices keyed by address. */
    private final ConcurrentHashMap<String, JSONObject> discoveredDevices = new ConcurrentHashMap<>();

    /** Active GATT connections keyed by device address. */
    private final ConcurrentHashMap<String, BluetoothGatt> gattConnections = new ConcurrentHashMap<>();

    /** Cached negotiated MTU per device. Updated by onMtuChanged callback. */
    private final ConcurrentHashMap<String, Integer> negotiatedMtu = new ConcurrentHashMap<>();

    /** Cached RSSI per device. Updated by onReadRemoteRssi callback. */
    private final ConcurrentHashMap<String, Integer> cachedRssi = new ConcurrentHashMap<>();

    /** Client Characteristic Configuration Descriptor UUID for enabling notifications. */
    private static final UUID CCCD_UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb");

    private BluetoothLeScanner leScanner;
    private ScanCallback leScanCallback;
    private BroadcastReceiver adapterStateReceiver;
    private final LifecycleRequestState<String> discoveryRequest;
    private final LifecycleRequestState<String> beaconRequest;

    public BluetoothManager(int sessionId, Activity activity) {
        this(sessionId, activity, false);
    }

    public BluetoothManager(int sessionId, Activity activity, boolean lifecycleSuspended) {
        this.sessionId = sessionId;
        this.activityRef = new WeakReference<>(activity);
        this.adapter = getAdapter(activity);
        this.discoveryRequest = new LifecycleRequestState<>(lifecycleSuspended);
        this.beaconRequest = new LifecycleRequestState<>(lifecycleSuspended);
    }

    private Activity getActivity() {
        return activityRef.get();
    }

    private static BluetoothAdapter getAdapter(Context context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            android.bluetooth.BluetoothManager bm = (android.bluetooth.BluetoothManager)
                    context.getSystemService(Context.BLUETOOTH_SERVICE);
            return bm != null ? bm.getAdapter() : null;
        }
        return BluetoothAdapter.getDefaultAdapter();
    }

    // ==================== Adapter ====================

    public void openAdapter(String optionsJson) {
        if (adapter == null) {
            throw new RuntimeException("openBluetoothAdapter:fail not available");
        }
        if (!adapter.isEnabled()) {
            throw new RuntimeException("openBluetoothAdapter:fail not available");
        }
        adapterOpened = true;
        registerAdapterStateReceiver();
    }

    public void closeAdapter() {
        adapterOpened = false;
        stopDiscovery();
        unregisterAdapterStateReceiver();
        discoveredDevices.clear();
        // Close all GATT connections when adapter is closed
        for (BluetoothGatt gatt : gattConnections.values()) {
            try {
                gatt.disconnect();
                gatt.close();
            } catch (Exception ignored) {}
        }
        gattConnections.clear();
        negotiatedMtu.clear();
        cachedRssi.clear();
    }

    public String getAdapterState() {
        boolean available = adapter != null && adapter.isEnabled();
        JSONObject obj = new JSONObject();
        try {
            obj.put("discovering", discovering);
            obj.put("available", available);
        } catch (JSONException ignored) {}
        return obj.toString();
    }

    // ==================== Device Discovery ====================

    public synchronized void startDiscovery(String optionsJson) {
        if (adapter == null || !adapterOpened) {
            throw new RuntimeException("startBluetoothDevicesDiscovery:fail adapter not opened");
        }

        discoveredDevices.clear();
        LifecycleRequestState.Action action = discoveryRequest.requestStart(optionsJson);
        if (action == LifecycleRequestState.Action.NONE) {
            discovering = false;
            return;
        }
        if (action == LifecycleRequestState.Action.RESTART) {
            stopDiscoveryInternal();
        }

        try {
            startDiscoveryInternal(optionsJson);
            discovering = true;
            NativeMethods.onBluetoothAdapterStateChange(sessionId, adapter.isEnabled(), true);
        } catch (RuntimeException e) {
            stopDiscoveryInternal();
            discovering = false;
            discoveryRequest.startFailed(false);
            throw e;
        }
    }

    private void startDiscoveryInternal(String optionsJson) {
        if (adapter == null || !adapterOpened || !adapter.isEnabled()) {
            throw new RuntimeException("startBluetoothDevicesDiscovery:fail adapter not opened");
        }

        List<ScanFilter> filters = new ArrayList<>();
        int scanMode = ScanSettings.SCAN_MODE_BALANCED;

        try {
            JSONObject opts = new JSONObject(optionsJson);
            JSONArray services = opts.optJSONArray("services");
            if (services != null) {
                for (int i = 0; i < services.length(); i++) {
                    String uuid = services.getString(i);
                    try {
                        ScanFilter filter = new ScanFilter.Builder()
                                .setServiceUuid(ParcelUuid.fromString(uuid))
                                .build();
                        filters.add(filter);
                    } catch (Exception e) {
                        Log.w(TAG, "Invalid service UUID: " + uuid);
                    }
                }
            }
            String powerLevel = opts.optString("powerLevel", "medium");
            switch (powerLevel) {
                case "low":
                    scanMode = ScanSettings.SCAN_MODE_LOW_POWER;
                    break;
                case "high":
                    scanMode = ScanSettings.SCAN_MODE_LOW_LATENCY;
                    break;
                default:
                    scanMode = ScanSettings.SCAN_MODE_BALANCED;
                    break;
            }
        } catch (JSONException ignored) {}

        leScanner = adapter.getBluetoothLeScanner();
        if (leScanner == null) {
            throw new RuntimeException("startBluetoothDevicesDiscovery:fail scanner not available");
        }

        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(scanMode)
                .build();

        leScanCallback = new ScanCallback() {
            @Override
            public void onScanResult(int callbackType, ScanResult result) {
                synchronized (BluetoothManager.this) {
                    if (leScanCallback != this || !discoveryRequest.isActive()) return;
                    handleScanResult(result);
                }
            }

            @Override
            public void onBatchScanResults(List<ScanResult> results) {
                synchronized (BluetoothManager.this) {
                    if (leScanCallback != this || !discoveryRequest.isActive()) return;
                    for (ScanResult result : results) {
                        handleScanResult(result);
                    }
                }
            }

            @Override
            public void onScanFailed(int errorCode) {
                synchronized (BluetoothManager.this) {
                    if (leScanCallback != this || !discoveryRequest.isActive()) return;
                    Log.e(TAG, "BLE scan failed: " + errorCode);
                    leScanCallback = null;
                    discovering = false;
                    discoveryRequest.startFailed(false);
                    NativeMethods.onBluetoothAdapterStateChange(sessionId,
                            adapter.isEnabled(), false);
                }
            }
        };

        leScanner.startScan(filters.isEmpty() ? null : filters, settings, leScanCallback);
    }

    public synchronized void stopDiscovery() {
        boolean wasDiscovering = discovering;
        if (discoveryRequest.requestStop() == LifecycleRequestState.Action.STOP) {
            stopDiscoveryInternal();
            discovering = false;
        }
        if (wasDiscovering && adapter != null) {
            NativeMethods.onBluetoothAdapterStateChange(sessionId,
                    adapter.isEnabled(), false);
        }
    }

    private void stopDiscoveryInternal() {
        BluetoothLeScanner scanner = leScanner;
        ScanCallback callback = leScanCallback;
        leScanCallback = null;
        if (scanner != null && callback != null) {
            try {
                scanner.stopScan(callback);
            } catch (Exception e) {
                Log.w(TAG, "stopScan error: " + e.getMessage());
            }
        }
    }

    public String getDevices() {
        JSONArray arr = new JSONArray();
        for (JSONObject dev : discoveredDevices.values()) {
            arr.put(dev);
        }
        JSONObject result = new JSONObject();
        try {
            result.put("devices", arr);
        } catch (JSONException ignored) {}
        return result.toString();
    }

    public String getConnectedDevices(String optionsJson) {
        JSONArray arr = new JSONArray();
        // Android doesn't provide a direct way to get BLE connected devices by service UUID
        // without using BluetoothGatt. Return empty for now.
        JSONObject result = new JSONObject();
        try {
            result.put("devices", arr);
        } catch (JSONException ignored) {}
        return result.toString();
    }

    // ==================== Pairing ====================

    public void makePair(String optionsJson) {
        if (adapter == null) {
            throw new RuntimeException("makeBluetoothPair:fail not available");
        }
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            BluetoothDevice device = adapter.getRemoteDevice(deviceId);
            // createBond() is API 19+, safe since minSdk=21
            device.createBond();
        } catch (JSONException e) {
            throw new RuntimeException("makeBluetoothPair:fail invalid options");
        }
    }

    public void isDevicePaired(String optionsJson) {
        if (adapter == null) {
            throw new RuntimeException("isBluetoothDevicePaired:fail not available");
        }
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            Set<BluetoothDevice> bondedDevices = adapter.getBondedDevices();
            for (BluetoothDevice device : bondedDevices) {
                if (device.getAddress().equalsIgnoreCase(deviceId)) {
                    return; // paired
                }
            }
            throw new RuntimeException("isBluetoothDevicePaired:fail not paired");
        } catch (JSONException e) {
            throw new RuntimeException("isBluetoothDevicePaired:fail invalid options");
        }
    }

    // ==================== Beacon ====================

    // Beacon discovery reuses BLE scanning with iBeacon/Eddystone parsing.
    // For simplicity, we use a separate flag and parse manufacturer data.

    private volatile boolean beaconDiscovering = false;
    private BluetoothLeScanner beaconScanner;
    private ScanCallback beaconScanCallback;
    private final ConcurrentHashMap<String, JSONObject> discoveredBeacons = new ConcurrentHashMap<>();

    public synchronized void startBeaconDiscovery(String optionsJson) {
        if (adapter == null || !adapter.isEnabled()) {
            throw new RuntimeException("startBeaconDiscovery:fail not available");
        }

        discoveredBeacons.clear();
        LifecycleRequestState.Action action = beaconRequest.requestStart(optionsJson);
        if (action == LifecycleRequestState.Action.NONE) {
            beaconDiscovering = false;
            return;
        }
        if (action == LifecycleRequestState.Action.RESTART) {
            stopBeaconDiscoveryInternal();
        }

        try {
            startBeaconDiscoveryInternal();
            beaconDiscovering = true;
            NativeMethods.onBeaconServiceChange(sessionId, true, true);
        } catch (RuntimeException e) {
            stopBeaconDiscoveryInternal();
            beaconDiscovering = false;
            beaconRequest.startFailed(false);
            throw e;
        }
    }

    private void startBeaconDiscoveryInternal() {
        if (adapter == null || !adapter.isEnabled()) {
            throw new RuntimeException("startBeaconDiscovery:fail not available");
        }

        beaconScanner = adapter.getBluetoothLeScanner();
        if (beaconScanner == null) {
            throw new RuntimeException("startBeaconDiscovery:fail scanner not available");
        }

        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build();

        beaconScanCallback = new ScanCallback() {
            @Override
            public void onScanResult(int callbackType, ScanResult result) {
                synchronized (BluetoothManager.this) {
                    if (beaconScanCallback != this || !beaconRequest.isActive()) return;
                    handleBeaconResult(result);
                }
            }

            @Override
            public void onBatchScanResults(List<ScanResult> results) {
                synchronized (BluetoothManager.this) {
                    if (beaconScanCallback != this || !beaconRequest.isActive()) return;
                    for (ScanResult r : results) {
                        handleBeaconResult(r);
                    }
                }
            }

            @Override
            public void onScanFailed(int errorCode) {
                synchronized (BluetoothManager.this) {
                    if (beaconScanCallback != this || !beaconRequest.isActive()) return;
                    Log.e(TAG, "Beacon scan failed: " + errorCode);
                    beaconScanCallback = null;
                    beaconDiscovering = false;
                    beaconRequest.startFailed(false);
                    NativeMethods.onBeaconServiceChange(sessionId, false, false);
                }
            }
        };

        beaconScanner.startScan(null, settings, beaconScanCallback);
    }

    public synchronized void stopBeaconDiscovery() {
        boolean wasDiscovering = beaconDiscovering;
        if (beaconRequest.requestStop() == LifecycleRequestState.Action.STOP) {
            stopBeaconDiscoveryInternal();
            beaconDiscovering = false;
        }
        if (wasDiscovering) {
            NativeMethods.onBeaconServiceChange(sessionId,
                    adapter != null && adapter.isEnabled(), false);
        }
    }

    private void stopBeaconDiscoveryInternal() {
        BluetoothLeScanner scanner = beaconScanner;
        ScanCallback callback = beaconScanCallback;
        beaconScanCallback = null;
        if (scanner != null && callback != null) {
            try {
                scanner.stopScan(callback);
            } catch (Exception e) {
                Log.w(TAG, "stopBeaconScan error: " + e.getMessage());
            }
        }
    }

    public String getBeacons() {
        JSONArray arr = new JSONArray();
        for (JSONObject beacon : discoveredBeacons.values()) {
            arr.put(beacon);
        }
        JSONObject result = new JSONObject();
        try {
            result.put("beacons", arr);
        } catch (JSONException ignored) {}
        return result.toString();
    }

    // ==================== BLE GATT ====================

    public void createBLEConnection(String optionsJson) {
        if (adapter == null) {
            throw new RuntimeException("createBLEConnection:fail adapter not available");
        }
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");

            if (gattConnections.containsKey(deviceId)) {
                return; // already connected
            }

            BluetoothDevice device = adapter.getRemoteDevice(deviceId);
            Activity activity = getActivity();
            Context ctx = activity != null ? activity : null;
            if (ctx == null) {
                throw new RuntimeException("createBLEConnection:fail no context");
            }

            BluetoothGattCallback callback = new BluetoothGattCallback() {
                @Override
                public void onConnectionStateChange(BluetoothGatt gatt, int status, int newState) {
                    boolean connected = (newState == BluetoothProfile.STATE_CONNECTED);
                    if (connected) {
                        gattConnections.put(deviceId, gatt);
                        gatt.discoverServices();
                    } else {
                        gattConnections.remove(deviceId);
                        gatt.close();
                    }
                    NativeMethods.onBLEConnectionStateChange(sessionId, deviceId, connected);
                }

                @Override
                public void onServicesDiscovered(BluetoothGatt gatt, int status) {
                    // Services cached in the BluetoothGatt object, retrieved via getServices()
                }

                @Override
                public void onCharacteristicRead(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic, int status) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        byte[] value = characteristic.getValue();
                        if (value == null) value = new byte[0];
                        String serviceId = characteristic.getService().getUuid().toString();
                        String charId = characteristic.getUuid().toString();
                        NativeMethods.onBLECharacteristicValueChange(sessionId, deviceId, serviceId, charId, value);
                    }
                }

                @Override
                public void onCharacteristicChanged(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic) {
                    byte[] value = characteristic.getValue();
                    if (value == null) value = new byte[0];
                    String serviceId = characteristic.getService().getUuid().toString();
                    String charId = characteristic.getUuid().toString();
                    NativeMethods.onBLECharacteristicValueChange(sessionId, deviceId, serviceId, charId, value);
                }

                @Override
                public void onMtuChanged(BluetoothGatt gatt, int mtu, int status) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        negotiatedMtu.put(deviceId, mtu);
                        NativeMethods.onBLEMTUChange(sessionId, deviceId, mtu);
                    }
                }

                @Override
                public void onReadRemoteRssi(BluetoothGatt gatt, int rssi, int status) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        cachedRssi.put(deviceId, rssi);
                    }
                }
            };

            BluetoothGatt gatt = device.connectGatt(
                    ctx, false, callback, BluetoothDevice.TRANSPORT_LE);
            if (gatt == null) {
                throw new RuntimeException("createBLEConnection:fail connect failed");
            }
        } catch (JSONException e) {
            throw new RuntimeException("createBLEConnection:fail invalid options");
        } catch (IllegalArgumentException e) {
            throw new RuntimeException("createBLEConnection:fail invalid deviceId");
        }
    }

    public void closeBLEConnection(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            BluetoothGatt gatt = gattConnections.remove(deviceId);
            negotiatedMtu.remove(deviceId);
            cachedRssi.remove(deviceId);
            if (gatt != null) {
                gatt.disconnect();
                gatt.close();
            }
        } catch (JSONException e) {
            throw new RuntimeException("closeBLEConnection:fail invalid options");
        }
    }

    public String getBLEDeviceServices(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("getBLEDeviceServices:fail not connected");
            }
            List<BluetoothGattService> services = gatt.getServices();
            JSONArray arr = new JSONArray();
            for (BluetoothGattService svc : services) {
                JSONObject svcJson = new JSONObject();
                svcJson.put("uuid", svc.getUuid().toString());
                svcJson.put("isPrimary", svc.getType() == BluetoothGattService.SERVICE_TYPE_PRIMARY);
                arr.put(svcJson);
            }
            JSONObject result = new JSONObject();
            result.put("services", arr);
            return result.toString();
        } catch (JSONException e) {
            throw new RuntimeException("getBLEDeviceServices:fail invalid options");
        }
    }

    public String getBLEDeviceCharacteristics(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            String serviceId = opts.getString("serviceId");
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("getBLEDeviceCharacteristics:fail not connected");
            }
            BluetoothGattService svc = gatt.getService(UUID.fromString(serviceId));
            if (svc == null) {
                throw new RuntimeException("getBLEDeviceCharacteristics:fail service not found");
            }
            JSONArray arr = new JSONArray();
            for (BluetoothGattCharacteristic ch : svc.getCharacteristics()) {
                JSONObject chJson = new JSONObject();
                chJson.put("uuid", ch.getUuid().toString());
                JSONObject props = new JSONObject();
                int p = ch.getProperties();
                props.put("read", (p & BluetoothGattCharacteristic.PROPERTY_READ) != 0);
                props.put("write", (p & BluetoothGattCharacteristic.PROPERTY_WRITE) != 0);
                props.put("notify", (p & BluetoothGattCharacteristic.PROPERTY_NOTIFY) != 0);
                props.put("indicate", (p & BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0);
                chJson.put("properties", props);
                arr.put(chJson);
            }
            JSONObject result = new JSONObject();
            result.put("characteristics", arr);
            return result.toString();
        } catch (JSONException e) {
            throw new RuntimeException("getBLEDeviceCharacteristics:fail invalid options");
        }
    }

    public void readBLECharacteristicValue(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            String serviceId = opts.getString("serviceId");
            String characteristicId = opts.getString("characteristicId");
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("readBLECharacteristicValue:fail not connected");
            }
            BluetoothGattCharacteristic ch = findCharacteristic(gatt, serviceId, characteristicId);
            if (ch == null) {
                throw new RuntimeException("readBLECharacteristicValue:fail characteristic not found");
            }
            if (!gatt.readCharacteristic(ch)) {
                throw new RuntimeException("readBLECharacteristicValue:fail read request failed");
            }
            // Result delivered asynchronously via onCharacteristicRead callback
        } catch (JSONException e) {
            throw new RuntimeException("readBLECharacteristicValue:fail invalid options");
        }
    }

    @SuppressWarnings("deprecation")
    public void writeBLECharacteristicValue(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            String serviceId = opts.getString("serviceId");
            String characteristicId = opts.getString("characteristicId");
            String valueHex = opts.optString("value", "");
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("writeBLECharacteristicValue:fail not connected");
            }
            BluetoothGattCharacteristic ch = findCharacteristic(gatt, serviceId, characteristicId);
            if (ch == null) {
                throw new RuntimeException("writeBLECharacteristicValue:fail characteristic not found");
            }
            byte[] value = hexToBytes(valueHex);
            String writeType = opts.optString("writeType", "write");
            int writeTypeInt = "writeNoResponse".equals(writeType)
                    ? BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
                    : BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT;
            boolean ok;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                int result = gatt.writeCharacteristic(ch, value, writeTypeInt);
                ok = (result == BluetoothGatt.GATT_SUCCESS); // BluetoothStatusCodes.SUCCESS == 0
            } else {
                ch.setValue(value);
                ch.setWriteType(writeTypeInt);
                ok = gatt.writeCharacteristic(ch);
            }
            if (!ok) {
                throw new RuntimeException("writeBLECharacteristicValue:fail write request failed");
            }
        } catch (JSONException e) {
            throw new RuntimeException("writeBLECharacteristicValue:fail invalid options");
        }
    }

    @SuppressWarnings("deprecation")
    public void notifyBLECharacteristicValueChange(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            String serviceId = opts.getString("serviceId");
            String characteristicId = opts.getString("characteristicId");
            boolean state = opts.optBoolean("state", true);
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("notifyBLECharacteristicValueChange:fail not connected");
            }
            BluetoothGattCharacteristic ch = findCharacteristic(gatt, serviceId, characteristicId);
            if (ch == null) {
                throw new RuntimeException("notifyBLECharacteristicValueChange:fail characteristic not found");
            }
            if (!gatt.setCharacteristicNotification(ch, state)) {
                throw new RuntimeException("notifyBLECharacteristicValueChange:fail set notification failed");
            }
            // Write CCCD to enable/disable server-side notifications
            BluetoothGattDescriptor cccd = ch.getDescriptor(CCCD_UUID);
            if (cccd != null) {
                byte[] descriptorValue;
                if (state) {
                    int props = ch.getProperties();
                    if ((props & BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0) {
                        descriptorValue = BluetoothGattDescriptor.ENABLE_INDICATION_VALUE;
                    } else {
                        descriptorValue = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE;
                    }
                } else {
                    descriptorValue = BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE;
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    gatt.writeDescriptor(cccd, descriptorValue);
                } else {
                    cccd.setValue(descriptorValue);
                    gatt.writeDescriptor(cccd);
                }
            }
        } catch (JSONException e) {
            throw new RuntimeException("notifyBLECharacteristicValueChange:fail invalid options");
        }
    }

    public String getBLEDeviceRSSI(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("getBLEDeviceRSSI:fail not connected");
            }
            // Trigger async RSSI read - result cached in onReadRemoteRssi
            gatt.readRemoteRssi();
            // Return last cached value (0 if never read before)
            Integer rssi = cachedRssi.get(deviceId);
            JSONObject result = new JSONObject();
            result.put("RSSI", rssi != null ? rssi.intValue() : 0);
            return result.toString();
        } catch (JSONException e) {
            throw new RuntimeException("getBLEDeviceRSSI:fail invalid options");
        }
    }

    public void setBLEMTU(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            int mtu = opts.optInt("mtu", 23);
            BluetoothGatt gatt = gattConnections.get(deviceId);
            if (gatt == null) {
                throw new RuntimeException("setBLEMTU:fail not connected");
            }
            if (!gatt.requestMtu(mtu)) {
                throw new RuntimeException("setBLEMTU:fail request failed");
            }
            // Result delivered via onMtuChanged callback
        } catch (JSONException e) {
            throw new RuntimeException("setBLEMTU:fail invalid options");
        }
    }

    public String getBLEMTU(String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String deviceId = opts.getString("deviceId");
            // Return cached negotiated MTU, or BLE default (23) if not yet negotiated
            Integer mtu = negotiatedMtu.get(deviceId);
            JSONObject result = new JSONObject();
            result.put("mtu", mtu != null ? mtu.intValue() : 23);
            return result.toString();
        } catch (JSONException e) {
            throw new RuntimeException("getBLEMTU:fail invalid options");
        }
    }

    private BluetoothGattCharacteristic findCharacteristic(
            BluetoothGatt gatt, String serviceId, String characteristicId) {
        BluetoothGattService svc = gatt.getService(UUID.fromString(serviceId));
        if (svc == null) return null;
        return svc.getCharacteristic(UUID.fromString(characteristicId));
    }

    private static byte[] hexToBytes(String hex) {
        if (hex == null || hex.isEmpty()) return new byte[0];
        int len = hex.length();
        if (len % 2 != 0) {
            throw new IllegalArgumentException("writeBLECharacteristicValue:fail value hex string has odd length");
        }
        byte[] data = new byte[len / 2];
        for (int i = 0; i < len; i += 2) {
            int hi = Character.digit(hex.charAt(i), 16);
            int lo = Character.digit(hex.charAt(i + 1), 16);
            if (hi < 0 || lo < 0) {
                throw new IllegalArgumentException("writeBLECharacteristicValue:fail value contains invalid hex character");
            }
            data[i / 2] = (byte) ((hi << 4) + lo);
        }
        return data;
    }

    // ==================== Cleanup ====================

    public synchronized void setLifecycleSuspended(boolean suspended) {
        if (suspended) {
            suspendForLifecycle();
        } else {
            resumeForLifecycle();
        }
    }

    public synchronized void suspendForLifecycle() {
        if (discoveryRequest.suspend() == LifecycleRequestState.Action.STOP) {
            stopDiscoveryInternal();
            discovering = false;
        }
        if (beaconRequest.suspend() == LifecycleRequestState.Action.STOP) {
            stopBeaconDiscoveryInternal();
            beaconDiscovering = false;
        }
    }

    public synchronized void resumeForLifecycle() {
        if (discoveryRequest.resume() == LifecycleRequestState.Action.START) {
            try {
                startDiscoveryInternal(discoveryRequest.getRequest());
                discovering = true;
            } catch (RuntimeException e) {
                Log.w(TAG, "Failed to resume BLE discovery: " + e.getMessage());
                stopDiscoveryInternal();
                discovering = false;
                discoveryRequest.startFailed(false);
                NativeMethods.onBluetoothAdapterStateChange(sessionId,
                        adapter != null && adapter.isEnabled(), false);
            }
        }

        if (beaconRequest.resume() == LifecycleRequestState.Action.START) {
            try {
                startBeaconDiscoveryInternal();
                beaconDiscovering = true;
            } catch (RuntimeException e) {
                Log.w(TAG, "Failed to resume beacon discovery: " + e.getMessage());
                stopBeaconDiscoveryInternal();
                beaconDiscovering = false;
                beaconRequest.startFailed(false);
                NativeMethods.onBeaconServiceChange(sessionId,
                        adapter != null && adapter.isEnabled(), false);
            }
        }
    }

    public synchronized void destroy() {
        if (discoveryRequest.destroy() == LifecycleRequestState.Action.STOP) {
            stopDiscoveryInternal();
        }
        if (beaconRequest.destroy() == LifecycleRequestState.Action.STOP) {
            stopBeaconDiscoveryInternal();
        }
        discovering = false;
        beaconDiscovering = false;
        closeAdapter();
        stopBeaconDiscoveryInternal();
        discoveredBeacons.clear();
        // Close all GATT connections
        for (BluetoothGatt gatt : gattConnections.values()) {
            try {
                gatt.disconnect();
                gatt.close();
            } catch (Exception ignored) {}
        }
        gattConnections.clear();
        negotiatedMtu.clear();
        cachedRssi.clear();
    }

    // ==================== Internal ====================

    private void handleScanResult(ScanResult result) {
        BluetoothDevice device = result.getDevice();
        String address = device.getAddress();

        JSONObject devJson = new JSONObject();
        try {
            devJson.put("deviceId", address);
            devJson.put("name", device.getName() != null ? device.getName() : "");
            devJson.put("RSSI", result.getRssi());
            // advertisData as hex string
            if (result.getScanRecord() != null && result.getScanRecord().getBytes() != null) {
                devJson.put("advertisData", bytesToHex(result.getScanRecord().getBytes()));
            }
            devJson.put("advertisServiceUUIDs", getServiceUuids(result));
        } catch (JSONException ignored) {}

        discoveredDevices.put(address, devJson);

        // Notify JS
        JSONArray devicesArr = new JSONArray();
        devicesArr.put(devJson);
        NativeMethods.onBluetoothDeviceFound(sessionId, devicesArr.toString());
    }

    private void handleBeaconResult(ScanResult result) {
        if (result.getScanRecord() == null || result.getScanRecord().getBytes() == null) {
            return;
        }
        byte[] scanRecord = result.getScanRecord().getBytes();
        // Parse iBeacon format: manufacturer specific data with Apple company ID (0x004C)
        JSONObject beacon = parseIBeacon(scanRecord, result.getRssi());
        if (beacon != null) {
            String key;
            try {
                key = beacon.getString("uuid") + ":" + beacon.getInt("major") + ":" + beacon.getInt("minor");
            } catch (JSONException e) {
                return;
            }
            discoveredBeacons.put(key, beacon);

            JSONArray beaconsArr = new JSONArray();
            for (JSONObject b : discoveredBeacons.values()) {
                beaconsArr.put(b);
            }
            NativeMethods.onBeaconUpdate(sessionId, beaconsArr.toString());
        }
    }

    private JSONObject parseIBeacon(byte[] scanRecord, int rssi) {
        // iBeacon format: ... 0xFF 0x4C 0x00 0x02 0x15 [UUID 16 bytes] [Major 2] [Minor 2] [TX 1]
        for (int i = 0; i < scanRecord.length - 25; i++) {
            if ((scanRecord[i] & 0xFF) == 0xFF
                    && (scanRecord[i + 1] & 0xFF) == 0x4C
                    && (scanRecord[i + 2] & 0xFF) == 0x00
                    && (scanRecord[i + 3] & 0xFF) == 0x02
                    && (scanRecord[i + 4] & 0xFF) == 0x15) {
                try {
                    byte[] uuidBytes = new byte[16];
                    System.arraycopy(scanRecord, i + 5, uuidBytes, 0, 16);
                    String uuid = bytesToUuid(uuidBytes);
                    int major = ((scanRecord[i + 21] & 0xFF) << 8) | (scanRecord[i + 22] & 0xFF);
                    int minor = ((scanRecord[i + 23] & 0xFF) << 8) | (scanRecord[i + 24] & 0xFF);
                    int txPower = scanRecord[i + 25]; // signed byte

                    JSONObject beacon = new JSONObject();
                    beacon.put("uuid", uuid);
                    beacon.put("major", major);
                    beacon.put("minor", minor);
                    beacon.put("proximity", estimateProximity(rssi, txPower));
                    beacon.put("accuracy", estimateAccuracy(rssi, txPower));
                    beacon.put("rssi", rssi);
                    return beacon;
                } catch (Exception e) {
                    return null;
                }
            }
        }
        return null;
    }

    private static int estimateProximity(int rssi, int txPower) {
        double distance = estimateAccuracy(rssi, txPower);
        if (distance < 0) return 0; // unknown
        if (distance < 0.5) return 1; // immediate
        if (distance < 4.0) return 2; // near
        return 3; // far
    }

    private static double estimateAccuracy(int rssi, int txPower) {
        if (rssi == 0) return -1.0;
        double ratio = (double) rssi / txPower;
        if (ratio < 1.0) {
            return Math.pow(ratio, 10);
        }
        return 0.89976 * Math.pow(ratio, 7.7095) + 0.111;
    }

    private void registerAdapterStateReceiver() {
        if (adapterStateReceiver != null) return;
        Activity activity = getActivity();
        if (activity == null) return;
        adapterStateReceiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                if (BluetoothAdapter.ACTION_STATE_CHANGED.equals(intent.getAction())) {
                    int state = intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR);
                    boolean available = (state == BluetoothAdapter.STATE_ON);
                    NativeMethods.onBluetoothAdapterStateChange(sessionId, available, discovering);
                }
            }
        };
        IntentFilter filter = new IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            activity.registerReceiver(adapterStateReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            activity.registerReceiver(adapterStateReceiver, filter);
        }
    }

    private void unregisterAdapterStateReceiver() {
        if (adapterStateReceiver != null) {
            Activity activity = getActivity();
            if (activity != null) {
                try {
                    activity.unregisterReceiver(adapterStateReceiver);
                } catch (Exception ignored) {}
            }
            adapterStateReceiver = null;
        }
    }

    private static JSONArray getServiceUuids(ScanResult result) {
        JSONArray arr = new JSONArray();
        if (result.getScanRecord() != null && result.getScanRecord().getServiceUuids() != null) {
            for (ParcelUuid uuid : result.getScanRecord().getServiceUuids()) {
                arr.put(uuid.toString());
            }
        }
        return arr;
    }

    private static String bytesToHex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(String.format("%02x", b & 0xFF));
        }
        return sb.toString();
    }

    private static String bytesToUuid(byte[] bytes) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 16; i++) {
            sb.append(String.format("%02x", bytes[i] & 0xFF));
            if (i == 3 || i == 5 || i == 7 || i == 9) {
                sb.append('-');
            }
        }
        return sb.toString();
    }
}
