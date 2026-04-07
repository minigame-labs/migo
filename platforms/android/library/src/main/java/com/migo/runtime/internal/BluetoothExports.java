package com.migo.runtime.internal;

import android.app.Activity;

import com.migo.runtime.internal.platform.BluetoothManager;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Domain class for per-session Bluetooth manager delegation.
 *
 * @hide
 */
public final class BluetoothExports {

    private BluetoothExports() {}

    private static final Object sBluetoothLock = new Object();

    private static final ConcurrentHashMap<Integer, BluetoothManager> sBluetoothManagers =
            new ConcurrentHashMap<>();

    private static BluetoothManager getOrCreateBluetoothManager(int sessionId) {
        BluetoothManager existing = sBluetoothManagers.get(sessionId);
        if (existing != null) return existing;
        synchronized (sBluetoothLock) {
            existing = sBluetoothManagers.get(sessionId);
            if (existing != null) return existing;
            RuntimeContext ctx = RuntimeRegistry.get(sessionId);
            if (ctx == null) return null;
            Activity activity = ctx.getActivity();
            if (activity == null) return null;
            BluetoothManager mgr = new BluetoothManager(sessionId, activity);
            sBluetoothManagers.put(sessionId, mgr);
            return mgr;
        }
    }

    public static void bluetoothOpenAdapter(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;
        mgr.openAdapter(optionsJson);
    }

    public static void bluetoothCloseAdapter(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.closeAdapter();
        }
    }

    public static String bluetoothGetAdapterState(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getAdapterState();
        }
        return "{\"discovering\":false,\"available\":false}";
    }

    public static void bluetoothStartDevicesDiscovery(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;
        mgr.startDiscovery(optionsJson);
    }

    public static void bluetoothStopDevicesDiscovery(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopDiscovery();
        }
    }

    public static String bluetoothGetDevices(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getDevices();
        }
        return "{\"devices\":[]}";
    }

    public static String bluetoothGetConnectedDevices(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getConnectedDevices(optionsJson);
        }
        return "{\"devices\":[]}";
    }

    public static void bluetoothMakePair(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;
        mgr.makePair(optionsJson);
    }

    public static void bluetoothIsDevicePaired(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;
        mgr.isDevicePaired(optionsJson);
    }

    public static void bluetoothStartBeaconDiscovery(int sessionId, String optionsJson) {
        BluetoothManager mgr = getOrCreateBluetoothManager(sessionId);
        if (mgr == null) return;
        mgr.startBeaconDiscovery(optionsJson);
    }

    public static void bluetoothStopBeaconDiscovery(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            mgr.stopBeaconDiscovery();
        }
    }

    public static String bluetoothGetBeacons(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr != null) {
            return mgr.getBeacons();
        }
        return "{\"beacons\":[]}";
    }

    // ---- BLE GATT ----

    public static void bleCreateConnection(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("createBLEConnection:fail no bluetooth manager");
        mgr.createBLEConnection(optionsJson);
    }

    public static void bleCloseConnection(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("closeBLEConnection:fail no bluetooth manager");
        mgr.closeBLEConnection(optionsJson);
    }

    public static String bleGetDeviceServices(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceServices:fail no bluetooth manager");
        return mgr.getBLEDeviceServices(optionsJson);
    }

    public static String bleGetDeviceCharacteristics(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceCharacteristics:fail no bluetooth manager");
        return mgr.getBLEDeviceCharacteristics(optionsJson);
    }

    public static void bleReadCharacteristicValue(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("readBLECharacteristicValue:fail no bluetooth manager");
        mgr.readBLECharacteristicValue(optionsJson);
    }

    public static void bleWriteCharacteristicValue(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("writeBLECharacteristicValue:fail no bluetooth manager");
        mgr.writeBLECharacteristicValue(optionsJson);
    }

    public static void bleNotifyCharacteristicValueChange(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("notifyBLECharacteristicValueChange:fail no bluetooth manager");
        mgr.notifyBLECharacteristicValueChange(optionsJson);
    }

    public static String bleGetDeviceRSSI(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEDeviceRSSI:fail no bluetooth manager");
        return mgr.getBLEDeviceRSSI(optionsJson);
    }

    public static void bleSetMTU(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("setBLEMTU:fail no bluetooth manager");
        mgr.setBLEMTU(optionsJson);
    }

    public static String bleGetMTU(int sessionId, String optionsJson) {
        BluetoothManager mgr = sBluetoothManagers.get(sessionId);
        if (mgr == null) throw new RuntimeException("getBLEMTU:fail no bluetooth manager");
        return mgr.getBLEMTU(optionsJson);
    }

    public static void destroyBluetoothManager(int sessionId) {
        BluetoothManager mgr = sBluetoothManagers.remove(sessionId);
        if (mgr != null) {
            mgr.destroy();
        }
    }

    public static void destroyAll(int sessionId) {
        destroyBluetoothManager(sessionId);
    }
}
