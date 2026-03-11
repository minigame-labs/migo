package com.migo.runtime.internal.platform;

/**
 * WGS-84 to GCJ-02 (China Mars coordinate) converter.
 *
 * <p>GCJ-02 is the mandatory coordinate system used by mapping services in China.
 * This implementation uses the standard offset algorithm that is well-documented
 * and widely used in open-source projects.
 *
 * <p>Points outside China are returned as-is (no offset applied).
 */
final class CoordinateConverter {

    private static final double PI = Math.PI;
    private static final double SEMI_MAJOR = 6378245.0;        // Krasovsky 1940 semi-major axis
    private static final double ECCENTRICITY_SQ = 0.00669342162296594323; // first eccentricity squared

    private CoordinateConverter() {}

    /**
     * Convert WGS-84 coordinates to GCJ-02.
     *
     * @param wgsLat WGS-84 latitude
     * @param wgsLng WGS-84 longitude
     * @return double[] { gcjLat, gcjLng }
     */
    static double[] wgs84ToGcj02(double wgsLat, double wgsLng) {
        if (isOutOfChina(wgsLat, wgsLng)) {
            return new double[]{wgsLat, wgsLng};
        }

        double dLat = transformLat(wgsLng - 105.0, wgsLat - 35.0);
        double dLng = transformLng(wgsLng - 105.0, wgsLat - 35.0);

        double radLat = wgsLat / 180.0 * PI;
        double magic = Math.sin(radLat);
        magic = 1 - ECCENTRICITY_SQ * magic * magic;
        double sqrtMagic = Math.sqrt(magic);

        dLat = (dLat * 180.0) / ((SEMI_MAJOR * (1 - ECCENTRICITY_SQ)) / (magic * sqrtMagic) * PI);
        dLng = (dLng * 180.0) / (SEMI_MAJOR / sqrtMagic * Math.cos(radLat) * PI);

        return new double[]{wgsLat + dLat, wgsLng + dLng};
    }

    private static double transformLat(double x, double y) {
        double ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y
                + 0.1 * x * y + 0.2 * Math.sqrt(Math.abs(x));
        ret += (20.0 * Math.sin(6.0 * x * PI) + 20.0 * Math.sin(2.0 * x * PI)) * 2.0 / 3.0;
        ret += (20.0 * Math.sin(y * PI) + 40.0 * Math.sin(y / 3.0 * PI)) * 2.0 / 3.0;
        ret += (160.0 * Math.sin(y / 12.0 * PI) + 320.0 * Math.sin(y * PI / 30.0)) * 2.0 / 3.0;
        return ret;
    }

    private static double transformLng(double x, double y) {
        double ret = 300.0 + x + 2.0 * y + 0.1 * x * x
                + 0.1 * x * y + 0.1 * Math.sqrt(Math.abs(x));
        ret += (20.0 * Math.sin(6.0 * x * PI) + 20.0 * Math.sin(2.0 * x * PI)) * 2.0 / 3.0;
        ret += (20.0 * Math.sin(x * PI) + 40.0 * Math.sin(x / 3.0 * PI)) * 2.0 / 3.0;
        ret += (150.0 * Math.sin(x / 12.0 * PI) + 300.0 * Math.sin(x / 30.0 * PI)) * 2.0 / 3.0;
        return ret;
    }

    /**
     * Rough bounding-box check for mainland China.
     * Points outside this box are not offset.
     */
    private static boolean isOutOfChina(double lat, double lng) {
        return lng < 72.004 || lng > 137.8347 || lat < 0.8293 || lat > 55.8271;
    }
}
