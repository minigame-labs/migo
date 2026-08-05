package com.migo.runtime.internal.platform;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/** Host-JVM tests for Bluetooth permission transitions across Android releases. */
public final class BluetoothPermissionPolicyTest {
    @Test
    public void connectPermissionStartsAtAndroid12() {
        assertTrue(BluetoothPermissionPolicy.canConnect(30, false));
        assertFalse(BluetoothPermissionPolicy.canConnect(31, false));
        assertTrue(BluetoothPermissionPolicy.canConnect(31, true));
        assertTrue(BluetoothPermissionPolicy.canConnect(34, true));
    }

    @Test
    public void scanUsesLocationThroughAndroid11() {
        assertFalse(BluetoothPermissionPolicy.canScan(26, false, false));
        assertTrue(BluetoothPermissionPolicy.canScan(30, false, true));
        assertFalse(BluetoothPermissionPolicy.canScan(30, true, false));
    }

    @Test
    public void scanUsesDedicatedPermissionFromAndroid12() {
        assertFalse(BluetoothPermissionPolicy.canScan(31, false, true));
        assertTrue(BluetoothPermissionPolicy.canScan(31, true, false));
        assertTrue(BluetoothPermissionPolicy.canScan(34, true, false));
    }
}
