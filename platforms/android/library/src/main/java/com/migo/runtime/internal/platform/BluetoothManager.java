package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
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

    private BluetoothLeScanner leScanner;
    private ScanCallback leScanCallback;
    private BroadcastReceiver adapterStateReceiver;

    public BluetoothManager(int sessionId, Activity activity) {
        this.sessionId = sessionId;
        this.activityRef = new WeakReference<>(activity);
        this.adapter = getAdapter(activity);
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

    public void startDiscovery(String optionsJson) {
        if (adapter == null || !adapterOpened) {
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

        stopDiscoveryInternal();
        discoveredDevices.clear();

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
                handleScanResult(result);
            }

            @Override
            public void onBatchScanResults(List<ScanResult> results) {
                for (ScanResult result : results) {
                    handleScanResult(result);
                }
            }

            @Override
            public void onScanFailed(int errorCode) {
                Log.e(TAG, "BLE scan failed: " + errorCode);
                discovering = false;
                NativeMethods.onBluetoothAdapterStateChange(sessionId,
                        adapter.isEnabled(), false);
            }
        };

        leScanner.startScan(filters.isEmpty() ? null : filters, settings, leScanCallback);
        discovering = true;
        NativeMethods.onBluetoothAdapterStateChange(sessionId, adapter.isEnabled(), true);
    }

    public void stopDiscovery() {
        stopDiscoveryInternal();
        if (discovering) {
            discovering = false;
            if (adapter != null) {
                NativeMethods.onBluetoothAdapterStateChange(sessionId,
                        adapter.isEnabled(), false);
            }
        }
    }

    private void stopDiscoveryInternal() {
        if (leScanner != null && leScanCallback != null) {
            try {
                leScanner.stopScan(leScanCallback);
            } catch (Exception e) {
                Log.w(TAG, "stopScan error: " + e.getMessage());
            }
            leScanCallback = null;
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

    public void startBeaconDiscovery(String optionsJson) {
        if (adapter == null || !adapter.isEnabled()) {
            throw new RuntimeException("startBeaconDiscovery:fail not available");
        }

        stopBeaconDiscoveryInternal();
        discoveredBeacons.clear();

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
                handleBeaconResult(result);
            }

            @Override
            public void onBatchScanResults(List<ScanResult> results) {
                for (ScanResult r : results) {
                    handleBeaconResult(r);
                }
            }

            @Override
            public void onScanFailed(int errorCode) {
                Log.e(TAG, "Beacon scan failed: " + errorCode);
                beaconDiscovering = false;
                NativeMethods.onBeaconServiceChange(sessionId, false, false);
            }
        };

        beaconScanner.startScan(null, settings, beaconScanCallback);
        beaconDiscovering = true;
        NativeMethods.onBeaconServiceChange(sessionId, true, true);
    }

    public void stopBeaconDiscovery() {
        stopBeaconDiscoveryInternal();
        if (beaconDiscovering) {
            beaconDiscovering = false;
            NativeMethods.onBeaconServiceChange(sessionId,
                    adapter != null && adapter.isEnabled(), false);
        }
    }

    private void stopBeaconDiscoveryInternal() {
        if (beaconScanner != null && beaconScanCallback != null) {
            try {
                beaconScanner.stopScan(beaconScanCallback);
            } catch (Exception e) {
                Log.w(TAG, "stopBeaconScan error: " + e.getMessage());
            }
            beaconScanCallback = null;
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

    // ==================== Cleanup ====================

    public void destroy() {
        closeAdapter();
        stopBeaconDiscoveryInternal();
        beaconDiscovering = false;
        discoveredBeacons.clear();
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
