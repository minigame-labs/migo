package com.migo.runtime.internal.platform;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkInfo;
import android.net.NetworkRequest;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.telephony.TelephonyManager;
import android.text.TextUtils;

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
    private final Context context;
    private final ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;

    public NetworkMonitor(int sessionId, Context context) {
        this.sessionId = sessionId;
        this.context = context;
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
            return new NetworkStatus(false, "none", "ConnectivityManager is null");
        }

        try {
            String networkType = "none";
            boolean isConnected = false;
            int signalStrength = 0;
            boolean hasSystemProxy = checkSystemProxy();
            boolean weakNet = false;

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                Network network = connectivityManager.getActiveNetwork();
                if (network != null) {
                    NetworkCapabilities capabilities = connectivityManager.getNetworkCapabilities(network);
                    if (capabilities != null) {
                        networkType = getNetworkTypeFromCapabilities(capabilities);
                        isConnected = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                                && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED);
                        
                        // Calculate signal strength and weak net status
                        if ("wifi".equals(networkType)) {
                            signalStrength = getWifiSignalStrength();
                        } else {
                            // TODO: Implement cellular signal strength
                            signalStrength = 0; 
                        }
                    }
                }
            } else {
                // Fallback for API 21-22
                NetworkInfo activeNetwork = connectivityManager.getActiveNetworkInfo();
                if (activeNetwork != null && activeNetwork.isConnected()) {
                    networkType = getNetworkTypeFromNetworkInfo(activeNetwork);
                    isConnected = true;
                    if ("wifi".equals(networkType)) {
                         signalStrength = getWifiSignalStrength();
                    }
                }
            }

            // Simple heuristic for weak network based on signal strength
            // This is a simplified logic, real implementation might be more complex
            if (isConnected && "wifi".equals(networkType) && signalStrength < -85) {
                weakNet = true;
            }

            return new NetworkStatus(isConnected, networkType, signalStrength, hasSystemProxy, weakNet, null);

        } catch (SecurityException e) {
            return new NetworkStatus(false, "none", "getNetworkType:fail:no permission (android.permission.ACCESS_NETWORK_STATE)");
        } catch (Exception e) {
            return new NetworkStatus(false, "none", "getNetworkType:fail:" + e.getMessage());
        }
    }

    private int getWifiSignalStrength() {
        try {
            WifiManager wifiManager = (WifiManager) context.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            WifiInfo wifiInfo = wifiManager.getConnectionInfo();
            if (wifiInfo != null) {
                return wifiInfo.getRssi();
            }
        } catch (Exception e) {
            android.util.Log.e("NetworkMonitor", "getWifiSignalStrength failed", e);
        }
        return 0;
    }

    private boolean checkSystemProxy() {
        try {
            String host = System.getProperty("http.proxyHost");
            String port = System.getProperty("http.proxyPort");
            return !TextUtils.isEmpty(host) && !TextUtils.isEmpty(port);
        } catch (Exception e) {
            return false;
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
                        return new LocalIPInfo(localip, netmask, null);
                    }
                }
            }
        } catch (SecurityException e) {
            return new LocalIPInfo("", "", "getLocalIPAddress:fail:no permission (INTERNET)");
        } catch (Exception e) {
            return new LocalIPInfo("", "", "getLocalIPAddress:fail:" + e.getMessage());
        }
        return new LocalIPInfo("", "", null);
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
        public final int signalStrength;
        public final boolean hasSystemProxy;
        public final boolean weakNet;
        public final String error;

        public NetworkStatus(boolean isConnected, String networkType, int signalStrength, boolean hasSystemProxy, boolean weakNet, String error) {
            this.isConnected = isConnected;
            this.networkType = networkType;
            this.signalStrength = signalStrength;
            this.hasSystemProxy = hasSystemProxy;
            this.weakNet = weakNet;
            this.error = error;
        }

        // Keep old constructor for compatibility if needed or update callers
        public NetworkStatus(boolean isConnected, String networkType, String error) {
            this(isConnected, networkType, 0, false, false, error);
        }
    }

    /**
     * Local IP info data class.
     */
    public static class LocalIPInfo {
        public final String localip;
        public final String netmask;
        public final String error;

        public LocalIPInfo(String localip, String netmask, String error) {
            this.localip = localip;
            this.netmask = netmask;
            this.error = error;
        }
    }
}
