package com.minigame.host.internal.device;

import android.content.Context;
import android.os.Build;
import android.util.Log; // added

public final class DeviceInfoHelper {

    private static String cachedJson;
    private static long cachedAt;

    private static final long CACHE_TTL_MS = 30_000;

    private DeviceInfoHelper() {
    }

    public static String getAsJson(Context context) {
        long now = System.currentTimeMillis();

        if (cachedJson != null && (now - cachedAt) < CACHE_TTL_MS) {
            Log.d("DeviceInfoHelper", "getAsJson: cache hit, returning cached JSON");
            return cachedJson;
        }

        Log.d("DeviceInfoHelper", "getAsJson: cache miss, rebuilding device info JSON");
        cachedJson = buildDeviceInfoJson(context.getApplicationContext());
        cachedAt = now;
        Log.d("DeviceInfoHelper", "getAsJson: built JSON length=" + (cachedJson != null ? cachedJson.length() : 0));
        return cachedJson;
    }

    // ------------------------------------------------------------------------
    // Build
    // ------------------------------------------------------------------------

    private static String buildDeviceInfoJson(Context ctx) {

        // Preferred ABI for native side
        String deviceAbi = getPrimaryAbi();
        String legacyAbi = safeString(Build.CPU_ABI);

        String brand = safeString(Build.BRAND);
        String model = safeString(Build.MODEL);
        if (model.isEmpty()) model = "unknown";

        String system = "Android " + safeString(Build.VERSION.RELEASE);
        String hardware = safeString(Build.HARDWARE);

        // cpuType is informational, not ABI contract
        String cpuType = !hardware.isEmpty() ? hardware : deviceAbi;

        long memorySizeMB = MemoryHelper.getTotalMemoryMB(ctx);

        Log.d("DeviceInfoHelper", "buildDeviceInfoJson: deviceAbi=" + deviceAbi + " legacyAbi=" + legacyAbi + " brand=" + brand + " model=" + model + " memoryMB=" + memorySizeMB);

        // Placeholder for legacy protocol compatibility
        int benchmarkLevel = 25;

        StringBuilder sb = new StringBuilder(224);
        sb.append('{');
        sb.append("\"abi\":\"").append(escape(legacyAbi)).append("\",");
        sb.append("\"deviceAbi\":\"").append(escape(deviceAbi)).append("\",");
        sb.append("\"benchmarkLevel\":").append(benchmarkLevel).append(',');
        sb.append("\"brand\":\"").append(escape(brand)).append("\",");
        sb.append("\"model\":\"").append(escape(model)).append("\",");
        sb.append("\"system\":\"").append(escape(system)).append("\",");
        sb.append("\"platform\":\"android\",");
        sb.append("\"cpuType\":\"").append(escape(cpuType)).append("\",");
        sb.append("\"memorySize\":").append(memorySizeMB);
        sb.append('}');
        return sb.toString();
    }

    // ------------------------------------------------------------------------
    // ABI
    // ------------------------------------------------------------------------

    private static String getPrimaryAbi() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            String[] abis = Build.SUPPORTED_ABIS;
            if (abis != null && abis.length > 0) {
                String res = safeString(abis[0]);
                Log.d("DeviceInfoHelper", "getPrimaryAbi: " + res);
                return res;
            }
        }
        String res = safeString(Build.CPU_ABI);
        Log.d("DeviceInfoHelper", "getPrimaryAbi (fallback): " + res);
        return res;
    }

    // ------------------------------------------------------------------------
    // Utils
    // ------------------------------------------------------------------------

    private static String safeString(String s) {
        return s == null ? "" : s;
    }

    /**
     * Minimal & fast JSON escaping.
     */
    private static String escape(String s) {
        if (s == null || s.isEmpty()) return "";

        StringBuilder out = null;
        int len = s.length();

        for (int i = 0; i < len; i++) {
            char c = s.charAt(i);
            String esc = null;

            switch (c) {
                case '\\':
                    esc = "\\\\";
                    break;
                case '"':
                    esc = "\\\"";
                    break;
                case '\b':
                    esc = "\\b";
                    break;
                case '\f':
                    esc = "\\f";
                    break;
                case '\n':
                    esc = "\\n";
                    break;
                case '\r':
                    esc = "\\r";
                    break;
                case '\t':
                    esc = "\\t";
                    break;
                default:
                    if (c <= 0x1F) {
                        esc = "\\u00" + hex(c >> 4) + hex(c);
                    }
            }

            if (esc != null) {
                if (out == null) {
                    out = new StringBuilder(len + 8);
                    if (i > 0) out.append(s, 0, i);
                }
                out.append(esc);
            } else if (out != null) {
                out.append(c);
            }
        }

        return out == null ? s : out.toString();
    }

    private static char hex(int v) {
        return (char) (v < 10 ? '0' + v : 'a' + (v - 10));
    }
}
