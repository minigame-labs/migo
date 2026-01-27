package com.migo.host.internal.bluetooth;

import android.app.Activity;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothManager;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.provider.Settings;
import android.util.Log;

import com.migo.host.internal.jni.HostNative;

public class BluetoothSettingActivity extends Activity {
    private static final int REQUEST_BLUETOOTH_SETTINGS = 10086;
    private BluetoothAdapter bluetoothAdapter;

    private int hostId;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Intent intent = getIntent();

        if (intent != null) {
            hostId = intent.getIntExtra("com.migo.host.EXTRA_HOST_ID", -1);
            if (hostId == -1) {
                Log.w("BluetoothSetting", "hostId not provided");
            }
        }

        try {
            BluetoothManager bluetoothManager = (BluetoothManager) getSystemService(Context.BLUETOOTH_SERVICE);
            if (bluetoothManager != null) {
                bluetoothAdapter = bluetoothManager.getAdapter();
            }
            if (bluetoothAdapter == null) {
                onFinish(0);
                return;
            }
        } catch (Exception e) {
            Log.e("Bluetooth", "Failed to get BluetoothManager", e);
        }

        openSystemBluetoothSettings();
    }

    private void openSystemBluetoothSettings() {
        Intent intent = new Intent(Settings.ACTION_BLUETOOTH_SETTINGS);
        startActivityForResult(intent, REQUEST_BLUETOOTH_SETTINGS);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {

        super.onActivityResult(requestCode, resultCode, data);

        if (requestCode == REQUEST_BLUETOOTH_SETTINGS) {
            checkBluetoothState();
        }
    }

    private void checkBluetoothState() {
        int bluetoothEnabled = 0;
        if (bluetoothAdapter != null) {
            bluetoothEnabled = bluetoothAdapter.isEnabled() ? 1 : 0;
        }

        onFinish(bluetoothEnabled);
    }

    private void onFinish(int code) {
        HostNative.onBluetoothSettingResult(hostId, code != 0);
        finish();
    }
}
