use std::borrow::Cow;
use std::fmt;

/// Standard error codes for service operations.
///
/// Each code maps to a JS `errCode` value and a human-readable category.
/// Domain-specific codes start at 10+ to leave room for generic codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ServiceErrorCode {
    NotSupported = 0,
    InvalidParam = 1,
    Timeout = 2,
    PermissionDenied = 3,
    SystemError = 4,
    Cancelled = 5,
}

/// Structured error for service trait methods.
///
/// Replaces the ad-hoc `Err("apiName:fail reason".to_string())` pattern
/// with typed errors that can be converted to JS `{ errMsg, errCode }`.
#[derive(Debug, Clone)]
pub struct ServiceError {
    pub code: ServiceErrorCode,
    pub message: String,
}

impl ServiceError {
    pub fn not_supported(msg: impl Into<String>) -> Self {
        Self {
            code: ServiceErrorCode::NotSupported,
            message: msg.into(),
        }
    }
    pub fn invalid_param(msg: impl Into<String>) -> Self {
        Self {
            code: ServiceErrorCode::InvalidParam,
            message: msg.into(),
        }
    }
    pub fn system(msg: impl Into<String>) -> Self {
        Self {
            code: ServiceErrorCode::SystemError,
            message: msg.into(),
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ServiceError {}

/// Conversion from String (for backward compatibility during migration)
impl From<String> for ServiceError {
    fn from(msg: String) -> Self {
        Self {
            code: ServiceErrorCode::SystemError,
            message: msg,
        }
    }
}
impl From<&str> for ServiceError {
    fn from(msg: &str) -> Self {
        Self {
            code: ServiceErrorCode::SystemError,
            message: msg.to_string(),
        }
    }
}

/// Allows `JsErrorBox::generic(service_error)` to work directly,
/// since `JsErrorBox::generic` accepts `impl Into<Cow<'static, str>>`.
impl From<ServiceError> for Cow<'static, str> {
    fn from(e: ServiceError) -> Self {
        Cow::Owned(e.message)
    }
}
