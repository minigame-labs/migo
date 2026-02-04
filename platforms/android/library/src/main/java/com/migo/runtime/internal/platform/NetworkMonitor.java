package com.migo.runtime.internal.platform;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkInfo;
import android.net.NetworkRequest;
import android.os.Build;
import android.telephony.TelephonyManager;

import com.migo.runtime.internal.NativeMethods;

import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.util.Enumeration;

/**
 * Network status monitoring utility.
 * <p>
 * Uses ConnectivityManager and NetworkCallback to monitor network changes.
 * Compatible with Android API 21+.
 *
 * @hide
 */
public final class NetworkMonitor {

    private final int sessionId;
    private final ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;

    public NetworkMonitor(int sessionId, Context context) {
        this.sessionId = sessionId;
        this.connectivityManager = (ConnectivityManager) context.getSystemService(Context.CONNECTIVITY_SERVICE);
    }

    /**
     * Start monitoring network status changes.
     */
    public void startMonitoring() {
        if (connectivityManager == null) return;
        stopMonitoring();

        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                notifyNetworkChange();
            }

            @Override
            public void onLost(Network network) {
                notifyNetworkChange();
            }

            @Override
            public void onCapabilitiesChanged(Network network, NetworkCapabilities capabilities) {
                notifyNetworkChange();
            }
        };

        NetworkRequest request = new NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .build();

        try {
            connectivityManager.registerNetworkCallback(request, networkCallback);
        } catch (Exception e) {
            // SecurityException if missing ACCESS_NETWORK_STATE permission
        }
    }

    /**
     * Stop monitoring network status changes.
     */
    public void stopMonitoring() {
        if (connectivityManager != null && networkCallback != null) {
            try {
                connectivityManager.unregisterNetworkCallback(networkCallback);
            } catch (Exception ignored) {
            }
            networkCallback = null;
        }
    }

    /**
     * Notify JS layer of network status change.
     */
    private void notifyNetworkChange() {
        NetworkStatus status = getNetworkStatus();
        NativeMethods.onNetworkStatusChange(sessionId, status.isConnected, status.networkType);
    }

    /**
     * Get current network status.
     */
    public NetworkStatus getNetworkStatus() {
        if (connectivityManager == null) {
            return new NetworkStatus(false, "none");
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            Network network = connectivityManager.getActiveNetwork();
            if (network == null) {
                return new NetworkStatus(false, "none");
            }

            NetworkCapabilities capabilities = connectivityManager.getNetworkCapabilities(network);
            if (capabilities == null) {
                return new NetworkStatus(false, "none");
            }

            String networkType = getNetworkTypeFromCapabilities(capabilities);
            boolean isConnected = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                    && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED);

            return new NetworkStatus(isConnected, networkType);
        } else {
            // Fallback for API 21-22
            NetworkInfo activeNetwork = connectivityManager.getActiveNetworkInfo();
            if (activeNetwork == null || !activeNetwork.isConnected()) {
                return new NetworkStatus(false, "none");
            }

            String networkType = getNetworkTypeFromNetworkInfo(activeNetwork);
            return new NetworkStatus(true, networkType);
        }
    }

    /**
     * Get network type string from NetworkCapabilities (API 23+).
     */
    private String getNetworkTypeFromCapabilities(NetworkCapabilities capabilities) {
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
            return "wifi";
        } else if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) {
            // For cellular, we need TelephonyManager to get specific type
            return "unknown"; // Simplified; could use TelephonyManager for 2g/3g/4g/5g
        } else if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
            return "wifi"; // Treat ethernet as wifi for simplicity
        } else {
            return "unknown";
        }
    }

    /**
     * Get network type string from NetworkInfo (API 21-22 fallback).
     */
    @SuppressWarnings("deprecation")
    private String getNetworkTypeFromNetworkInfo(NetworkInfo networkInfo) {
        int type = networkInfo.getType();
        if (type == ConnectivityManager.TYPE_WIFI) {
            return "wifi";
        } else if (type == ConnectivityManager.TYPE_MOBILE) {
            int subType = networkInfo.getSubtype();
            switch (subType) {
                case TelephonyManager.NETWORK_TYPE_GPRS:
                case TelephonyManager.NETWORK_TYPE_EDGE:
                case TelephonyManager.NETWORK_TYPE_CDMA:
                case TelephonyManager.NETWORK_TYPE_1xRTT:
                case TelephonyManager.NETWORK_TYPE_IDEN:
                    return "2g";
                case TelephonyManager.NETWORK_TYPE_UMTS:
                case TelephonyManager.NETWORK_TYPE_EVDO_0:
                case TelephonyManager.NETWORK_TYPE_EVDO_A:
                case TelephonyManager.NETWORK_TYPE_HSDPA:
                case TelephonyManager.NETWORK_TYPE_HSUPA:
                case TelephonyManager.NETWORK_TYPE_HSPA:
                case TelephonyManager.NETWORK_TYPE_EVDO_B:
                case TelephonyManager.NETWORK_TYPE_EHRPD:
                case TelephonyManager.NETWORK_TYPE_HSPAP:
                    return "3g";
                case TelephonyManager.NETWORK_TYPE_LTE:
                    return "4g";
                case TelephonyManager.NETWORK_TYPE_NR:
                    return "5g";
                default:
                    return "unknown";
            }
        } else if (type == ConnectivityManager.TYPE_ETHERNET) {
            return "wifi";
        }
        return "unknown";
    }

    /**
     * Get local IP address and netmask.
     *
     * @return LocalIPInfo with localip and netmask
     */
    public static LocalIPInfo getLocalIPAddress() {
        try {
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces.hasMoreElements()) {
                NetworkInterface networkInterface = interfaces.nextElement();
                if (networkInterface.isLoopback() || !networkInterface.isUp()) {
                    continue;
                }

                Enumeration<InetAddress> addresses = networkInterface.getInetAddresses();
                while (addresses.hasMoreElements()) {
                    InetAddress address = addresses.nextElement();
                    if (address instanceof Inet4Address && !address.isLoopbackAddress()) {
                        String localip = address.getHostAddress();
                        // Get netmask from interface prefix length
                        short prefixLength = networkInterface.getInterfaceAddresses().get(0).getNetworkPrefixLength();
                        String netmask = prefixToNetmask(prefixLength);
                        return new LocalIPInfo(localip, netmask);
                    }
                }
            }
        } catch (Exception e) {
            // SocketException or SecurityException
        }
        return new LocalIPInfo("", "");
    }

    /**
     * Convert prefix length to netmask string.
     */
    private static String prefixToNetmask(short prefixLength) {
        int mask = 0xFFFFFFFF << (32 - prefixLength);
        return String.format("%d.%d.%d.%d",
                (mask >> 24) & 0xFF,
                (mask >> 16) & 0xFF,
                (mask >> 8) & 0xFF,
                mask & 0xFF);
    }

    /**
     * Clean up resources.
     */
    public void destroy() {
        stopMonitoring();
    }

    /**
     * Network status data class.
     */
    public static class NetworkStatus {
        public final boolean isConnected;
        public final String networkType;

        public NetworkStatus(boolean isConnected, String networkType) {
            this.isConnected = isConnected;
            this.networkType = networkType;
        }
    }

    /**
     * Local IP info data class.
     */
    public static class LocalIPInfo {
        public final String localip;
        public final String netmask;

        public LocalIPInfo(String localip, String netmask) {
            this.localip = localip;
            this.netmask = netmask;
        }
    }
}
