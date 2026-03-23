//! Location service traits for GPS/network positioning.

use crate::protocol::error::ServiceError;

/// Location service for getting device geographic position.
///
/// Both methods are fire-and-forget: the Java side starts an async location
/// request and delivers the result via JNI inbound callback
/// (`onLocationResult` / `onFuzzyLocationResult` -> `EvalScript`).
pub trait LocationService: Send + Sync {
    /// Start a precise location request (getLocation).
    ///
    /// Options JSON fields:
    /// - `type`: "wgs84" (default) or "gcj02"
    /// - `altitude`: bool (default false) -- include altitude
    /// - `isHighAccuracy`: bool (default false)
    /// - `highAccuracyExpireTime`: number in ms
    fn get_location(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("getLocation:fail not supported"))
    }

    /// Start a fuzzy location request (getFuzzyLocation).
    ///
    /// Options JSON fields:
    /// - `type`: "wgs84" (default) or "gcj02"
    fn get_fuzzy_location(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("getFuzzyLocation:fail not supported"))
    }
}
