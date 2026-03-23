use crate::protocol::error::ServiceError;

/// Login/auth session service.
///
/// Host side should manage real session state (not JS memory state), and report
/// async results back via JNI callbacks:
/// - `NativeMethods.onLoginResult(sessionId, json)`
/// - `NativeMethods.onCheckSessionResult(sessionId, json)`
/// - `NativeMethods.onGetUserInfoResult(sessionId, json)`
/// - `NativeMethods.onGetPhoneNumberResult(sessionId, json)`
///
/// Request JSON fields (both methods):
/// - `requestId`: number, used to correlate async callback
///
/// `login` request JSON optional fields:
/// - `timeout`: number (ms)
///
/// `onLoginResult` callback JSON:
/// - success: `{"requestId":N,"code":"..."}`
/// - fail: `{"requestId":N,"error":"reason","errno":123}` (errno optional)
///
/// `onCheckSessionResult` callback JSON:
/// - success: `{"requestId":N}`
/// - fail: `{"requestId":N,"error":"reason","errno":123}` (errno optional)
///
/// `getUserInfo` request JSON optional fields:
/// - `withCredentials`: boolean
/// - `lang`: string (`en`, `zh_CN`, `zh_TW`)
///
/// `onGetUserInfoResult` callback JSON:
/// - success: `{"requestId":N,"userInfo":{...},"rawData":"...","signature":"...","encryptedData":"...","iv":"..."}`
/// - fail: `{"requestId":N,"error":"reason"}`
///
/// `getPhoneNumber` request JSON optional fields:
/// - `isRealtime`: boolean
/// - `phoneNumberNoQuotaToast`: boolean
///
/// `onGetPhoneNumberResult` callback JSON:
/// - success: `{"requestId":N,"code":"..."}`
/// - fail: `{"requestId":N,"error":"reason","errno":123}` (errno optional)
pub trait AuthService: Send + Sync {
    /// Trigger login and return code asynchronously.
    fn login(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("login:fail not supported"))
    }

    /// Check current session validity asynchronously.
    fn check_session(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("checkSession:fail not supported"))
    }

    /// Get user info asynchronously.
    fn get_user_info(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("getUserInfo:fail not supported"))
    }

    /// Get phone number token asynchronously.
    fn get_phone_number(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("getPhoneNumber:fail not supported"))
    }
}
