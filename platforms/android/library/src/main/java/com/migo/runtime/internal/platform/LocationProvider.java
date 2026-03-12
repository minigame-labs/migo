package com.migo.runtime.internal.platform;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONException;
import org.json.JSONObject;

/**
 * Async location provider for getLocation / getFuzzyLocation.
 *
 * <p>Design decisions for broad Android compatibility (API 21+, domestic & international):
 * <ul>
 *   <li>Uses {@link LocationManager} directly (not Google Play Services / FusedLocationProvider)
 *       to work on devices without GMS (Huawei, many domestic Chinese phones).</li>
 *   <li>Tries GPS_PROVIDER first for high accuracy, falls back to NETWORK_PROVIDER.</li>
 *   <li>Uses {@link LocationManager#getLastKnownLocation} first for a quick result,
 *       then requests a single async update if high accuracy is needed or
 *       last-known is stale (>60s).</li>
 *   <li>Coordinate conversion (WGS-84 to GCJ-02) is done on the Java side.</li>
 *   <li>Results are delivered via {@link NativeMethods} callbacks — no thread blocking.</li>
 * </ul>
 */
public final class LocationProvider {

    private static final long STALE_THRESHOLD_MS = 60_000; // 60s
    private static final long DEFAULT_TIMEOUT_MS = 10_000; // 10s default timeout
    private static final Handler MAIN_HANDLER = new Handler(Looper.getMainLooper());

    private LocationProvider() {}

    // ==================== Public async entry points ====================

    /**
     * Start an async location request (getLocation).
     * Result is delivered via {@link NativeMethods#onLocationResult}.
     *
     * @param context     Android context
     * @param sessionId   Session ID for the callback
     * @param optionsJson JSON with: type, altitude, isHighAccuracy, highAccuracyExpireTime
     */
    public static void getLocationAsync(Context context, int sessionId, String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String type = opts.optString("type", "wgs84");
            boolean altitude = opts.optBoolean("altitude", false);
            boolean isHighAccuracy = opts.optBoolean("isHighAccuracy", false);
            long highAccuracyExpireTime = opts.optLong("highAccuracyExpireTime", 0);

            if (!hasLocationPermission(context)) {
                NativeMethods.onLocationResult(sessionId,
                        errorJson("getLocation", "no location permission"));
                return;
            }

            LocationManager lm = (LocationManager) context.getSystemService(Context.LOCATION_SERVICE);
            if (lm == null) {
                NativeMethods.onLocationResult(sessionId,
                        errorJson("getLocation", "location service unavailable"));
                return;
            }

            long timeoutMs = DEFAULT_TIMEOUT_MS;
            if (isHighAccuracy && highAccuracyExpireTime > 0) {
                timeoutMs = highAccuracyExpireTime;
            }

            boolean useGps = isHighAccuracy || isProviderEnabled(lm, LocationManager.GPS_PROVIDER);
            boolean useNetwork = isProviderEnabled(lm, LocationManager.NETWORK_PROVIDER);

            if (!useGps && !useNetwork) {
                NativeMethods.onLocationResult(sessionId,
                        errorJson("getLocation", "location service disabled"));
                return;
            }

            // Try last known first
            Location lastKnown = getBestLastKnown(lm, useGps, useNetwork);

            boolean needFresh = isHighAccuracy || lastKnown == null || isStale(lastKnown);

            if (!needFresh && lastKnown != null) {
                // Fast path: return immediately
                NativeMethods.onLocationResult(sessionId,
                        buildLocationResult(lastKnown, type, altitude));
                return;
            }

            // Async path: request a single update with timeout
            requestSingleUpdateAsync(lm, useGps, useNetwork, timeoutMs, lastKnown,
                    (location) -> {
                        if (location != null) {
                            NativeMethods.onLocationResult(sessionId,
                                    buildLocationResult(location, type, altitude));
                        } else {
                            NativeMethods.onLocationResult(sessionId,
                                    errorJson("getLocation", "unable to get location"));
                        }
                    });

        } catch (Exception e) {
            NativeMethods.onLocationResult(sessionId,
                    errorJson("getLocation", e.getMessage()));
        }
    }

    /**
     * Start an async fuzzy location request (getFuzzyLocation).
     * Result is delivered via {@link NativeMethods#onFuzzyLocationResult}.
     *
     * @param context     Android context
     * @param sessionId   Session ID for the callback
     * @param optionsJson JSON with: type
     */
    public static void getFuzzyLocationAsync(Context context, int sessionId, String optionsJson) {
        try {
            JSONObject opts = new JSONObject(optionsJson);
            String type = opts.optString("type", "wgs84");

            if (!hasLocationPermission(context)) {
                NativeMethods.onFuzzyLocationResult(sessionId,
                        errorJson("getFuzzyLocation", "no location permission"));
                return;
            }

            LocationManager lm = (LocationManager) context.getSystemService(Context.LOCATION_SERVICE);
            if (lm == null) {
                NativeMethods.onFuzzyLocationResult(sessionId,
                        errorJson("getFuzzyLocation", "location service unavailable"));
                return;
            }

            boolean useGps = isProviderEnabled(lm, LocationManager.GPS_PROVIDER);
            boolean useNetwork = isProviderEnabled(lm, LocationManager.NETWORK_PROVIDER);

            if (!useGps && !useNetwork) {
                NativeMethods.onFuzzyLocationResult(sessionId,
                        errorJson("getFuzzyLocation", "location service disabled"));
                return;
            }

            // Prefer network for fuzzy (lower accuracy is fine)
            Location lastKnown = null;
            if (useNetwork) {
                lastKnown = getLastKnownSafe(lm, LocationManager.NETWORK_PROVIDER);
            }
            if (lastKnown == null && useGps) {
                lastKnown = getLastKnownSafe(lm, LocationManager.GPS_PROVIDER);
            }

            if (lastKnown != null) {
                // Fast path
                NativeMethods.onFuzzyLocationResult(sessionId,
                        buildFuzzyResult(lastKnown, type));
                return;
            }

            // Async path
            requestSingleUpdateAsync(lm, false, useNetwork, DEFAULT_TIMEOUT_MS, null,
                    (location) -> {
                        if (location != null) {
                            NativeMethods.onFuzzyLocationResult(sessionId,
                                    buildFuzzyResult(location, type));
                        } else {
                            NativeMethods.onFuzzyLocationResult(sessionId,
                                    errorJson("getFuzzyLocation", "unable to get location"));
                        }
                    });

        } catch (Exception e) {
            NativeMethods.onFuzzyLocationResult(sessionId,
                    errorJson("getFuzzyLocation", e.getMessage()));
        }
    }

    // ==================== Async location request ====================

    private interface LocationCallback {
        void onResult(Location location);
    }

    /**
     * Request a single location update asynchronously.
     * The callback is invoked on the main thread when a location arrives or timeout expires.
     * No thread is blocked.
     */
    @SuppressWarnings("MissingPermission")
    private static void requestSingleUpdateAsync(
            LocationManager lm, boolean useGps, boolean useNetwork,
            long timeoutMs, Location fallback, LocationCallback callback) {

        final Location[] best = new Location[]{ null };
        final boolean[] done = new boolean[]{ false };

        LocationListener listener = new LocationListener() {
            @Override
            public void onLocationChanged(Location location) {
                if (done[0]) return;
                if (best[0] == null || isBetter(location, best[0])) {
                    best[0] = location;
                }
                // Deliver immediately on first fix
                done[0] = true;
                lm.removeUpdates(this);
                callback.onResult(best[0]);
            }

            @Override
            public void onStatusChanged(String provider, int status, Bundle extras) {}

            @Override
            public void onProviderEnabled(String provider) {}

            @Override
            public void onProviderDisabled(String provider) {
                if (done[0]) return;
                done[0] = true;
                lm.removeUpdates(this);
                callback.onResult(fallback);
            }
        };

        // Timeout: if no location received within timeoutMs, return fallback
        MAIN_HANDLER.postDelayed(() -> {
            if (done[0]) return;
            done[0] = true;
            try {
                lm.removeUpdates(listener);
            } catch (Exception ignored) {}
            callback.onResult(fallback);
        }, timeoutMs);

        // Request updates on main thread (requires Looper)
        MAIN_HANDLER.post(() -> {
            try {
                if (useGps && isProviderEnabled(lm, LocationManager.GPS_PROVIDER)) {
                    lm.requestSingleUpdate(LocationManager.GPS_PROVIDER, listener, Looper.getMainLooper());
                }
                if (useNetwork && isProviderEnabled(lm, LocationManager.NETWORK_PROVIDER)) {
                    lm.requestSingleUpdate(LocationManager.NETWORK_PROVIDER, listener, Looper.getMainLooper());
                }
            } catch (SecurityException e) {
                if (!done[0]) {
                    done[0] = true;
                    callback.onResult(fallback);
                }
            }
        });
    }

    // ==================== Result builders ====================

    private static String buildLocationResult(Location location, String type, boolean includeAltitude) {
        try {
            double lat = location.getLatitude();
            double lng = location.getLongitude();

            if ("gcj02".equals(type)) {
                double[] gcj = CoordinateConverter.wgs84ToGcj02(lat, lng);
                lat = gcj[0];
                lng = gcj[1];
            }

            JSONObject result = new JSONObject();
            result.put("latitude", lat);
            result.put("longitude", lng);
            result.put("speed", location.hasSpeed() ? location.getSpeed() : 0);
            result.put("accuracy", location.hasAccuracy() ? location.getAccuracy() : 0);

            if (includeAltitude) {
                result.put("altitude", location.hasAltitude() ? location.getAltitude() : 0);
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    result.put("verticalAccuracy",
                            location.hasVerticalAccuracy() ? location.getVerticalAccuracyMeters() : 0);
                } else {
                    result.put("verticalAccuracy", 0);
                }
                result.put("horizontalAccuracy", location.hasAccuracy() ? location.getAccuracy() : 0);
            } else {
                result.put("altitude", 0);
                result.put("verticalAccuracy", 0);
                result.put("horizontalAccuracy", location.hasAccuracy() ? location.getAccuracy() : 0);
            }

            return result.toString();
        } catch (JSONException e) {
            return errorJson("getLocation", e.getMessage());
        }
    }

    private static String buildFuzzyResult(Location location, String type) {
        try {
            double lat = location.getLatitude();
            double lng = location.getLongitude();

            // Apply fuzzing: round to ~1km precision (approx 0.01 degree)
            lat = Math.round(lat * 100.0) / 100.0;
            lng = Math.round(lng * 100.0) / 100.0;

            if ("gcj02".equals(type)) {
                double[] gcj = CoordinateConverter.wgs84ToGcj02(lat, lng);
                lat = gcj[0];
                lng = gcj[1];
            }

            JSONObject result = new JSONObject();
            result.put("latitude", lat);
            result.put("longitude", lng);
            return result.toString();
        } catch (JSONException e) {
            return errorJson("getFuzzyLocation", e.getMessage());
        }
    }

    // ==================== Utility helpers ====================

    private static Location getBestLastKnown(LocationManager lm, boolean useGps, boolean useNetwork) {
        Location gpsLoc = useGps ? getLastKnownSafe(lm, LocationManager.GPS_PROVIDER) : null;
        Location netLoc = useNetwork ? getLastKnownSafe(lm, LocationManager.NETWORK_PROVIDER) : null;

        if (gpsLoc == null) return netLoc;
        if (netLoc == null) return gpsLoc;
        return gpsLoc.getTime() >= netLoc.getTime() ? gpsLoc : netLoc;
    }

    @SuppressWarnings("MissingPermission")
    private static Location getLastKnownSafe(LocationManager lm, String provider) {
        try {
            return lm.getLastKnownLocation(provider);
        } catch (SecurityException | IllegalArgumentException e) {
            return null;
        }
    }

    private static boolean isBetter(Location newLoc, Location oldLoc) {
        if (oldLoc == null) return true;
        long timeDelta = newLoc.getTime() - oldLoc.getTime();
        if (timeDelta > 2000) return true;
        if (timeDelta < -2000) return false;
        return newLoc.hasAccuracy() && (!oldLoc.hasAccuracy() || newLoc.getAccuracy() < oldLoc.getAccuracy());
    }

    private static boolean isStale(Location location) {
        return System.currentTimeMillis() - location.getTime() > STALE_THRESHOLD_MS;
    }

    private static boolean isProviderEnabled(LocationManager lm, String provider) {
        try {
            return lm.isProviderEnabled(provider);
        } catch (Exception e) {
            return false;
        }
    }

    private static boolean hasLocationPermission(Context context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
            return true;
        }
        return context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)
                    == PackageManager.PERMISSION_GRANTED
                || context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION)
                    == PackageManager.PERMISSION_GRANTED;
    }

    private static String errorJson(String apiName, String reason) {
        return "{\"error\":\"" + apiName + ":fail " + escapeJson(reason) + "\"}";
    }

    private static String escapeJson(String s) {
        if (s == null) return "unknown error";
        return s.replace("\\", "\\\\").replace("\"", "\\\"");
    }
}
