package com.minigame.host.internal.system;

import android.bluetooth.BluetoothAdapter;
import android.content.Context;
import android.content.res.Configuration;
import android.location.LocationManager;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkInfo;
import android.net.wifi.WifiManager;
import android.os.Build;

public final class SystemSettingHelper {

    private SystemSettingHelper() { }

    public static byte[] getAsBytes(Context context) {
        boolean bluetoothEnabled = isBluetoothEnabled();
        boolean locationEnabled = isLocationEnabled(context);
        boolean wifiEnabled = isWifiConnected(context);
        int deviceOrientation = getDeviceOrientation(context);

        return new byte[]{
                (byte) (bluetoothEnabled ? 1 : 0),
                (byte) (locationEnabled ? 1 : 0),
                (byte) (wifiEnabled ? 1 : 0),
                (byte) deviceOrientation
        };
    }

    private static boolean isBluetoothEnabled() {
        try {
            BluetoothAdapter adapter = BluetoothAdapter.getDefaultAdapter();
            return adapter != null && adapter.isEnabled();
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static boolean isLocationEnabled(Context ctx) {
        if (ctx == null) return false;
        try {
            LocationManager lm = (LocationManager) ctx.getSystemService(Context.LOCATION_SERVICE);
            if (lm == null) return false;

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                return lm.isLocationEnabled();
            } else {
                boolean gps = false, network = false;
                try { gps = lm.isProviderEnabled(LocationManager.GPS_PROVIDER); } catch (Exception ignored) { }
                try { network = lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER); } catch (Exception ignored) { }
                return gps || network;
            }
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static boolean isWifiConnected(Context ctx) {
        if (ctx == null) return false;
        try {
            ConnectivityManager cm = (ConnectivityManager) ctx.getSystemService(Context.CONNECTIVITY_SERVICE);
            if (cm == null) return false;

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                Network nw = cm.getActiveNetwork();
                if (nw == null) return false;

                NetworkCapabilities caps = cm.getNetworkCapabilities(nw);
                return caps != null && caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI);
            } else {
                NetworkInfo info = cm.getActiveNetworkInfo();
                return info != null && info.isConnected() && info.getType() == ConnectivityManager.TYPE_WIFI;
            }
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static int getDeviceOrientation(Context ctx) {
        if (ctx == null) return 0;
        try {
            int orientation = ctx.getResources().getConfiguration().orientation;
            if (orientation == Configuration.ORIENTATION_PORTRAIT) return 1;
            if (orientation == Configuration.ORIENTATION_LANDSCAPE) return 2;
        } catch (Throwable ignored) { }
        return 0;
    }
}
