//! Location service traits for GPS/network positioning.

/// Location service for getting device geographic position.
///
/// Both methods are fire-and-forget: the Java side starts an async location
/// request and delivers the result via JNI inbound callback
/// (`onLocationResult` / `onFuzzyLocationResult` → `EvalScript`).
pub trait LocationService: Send + Sync {
    /// Start a precise location request (wx.getLocation).
    ///
    /// Options JSON fields:
    /// - `type`: "wgs84" (default) or "gcj02"
    /// - `altitude`: bool (default false) — include altitude
    /// - `isHighAccuracy`: bool (default false)
    /// - `highAccuracyExpireTime`: number in ms
    fn get_location(&self, _options_json: &str) -> Result<(), String> {
        Err("getLocation:fail not supported".to_string())
    }

    /// Start a fuzzy location request (wx.getFuzzyLocation).
    ///
    /// Options JSON fields:
    /// - `type`: "wgs84" (default) or "gcj02"
    fn get_fuzzy_location(&self, _options_json: &str) -> Result<(), String> {
        Err("getFuzzyLocation:fail not supported".to_string())
    }
}
