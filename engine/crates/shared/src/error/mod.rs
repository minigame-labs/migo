//! # Error Handling Module
//!
//! Provides unified error handling across the Migo engine with structured error codes
//! and detailed error information.
//!
//! ## Overview
//!
//! The error system consists of:
//!
//! - [`ErrorCode`]: Enumeration of all possible error categories
//! - [`EngineError`]: Rich error type with code, message, and optional details
//! - [`EngineResult<T>`]: Convenient type alias for `Result<T, EngineError>`
//!
//! ## Usage Examples
//!
//! ### Creating Errors
//!
//! ```rust,ignore
//! use shared::error::{EngineError, ErrorCode};
//!
//! // Simple error with default message
//! let err = EngineError::new(ErrorCode::NotFound);
//!
//! // Error with custom message
//! let err = EngineError::new(ErrorCode::InvalidArgument)
//!     .with_msg("invalid canvas ID");
//!
//! // Error with details for debugging
//! let err = EngineError::new(ErrorCode::IoError)
//!     .with_msg("failed to read file")
//!     .with_detail("path: /data/game.js, errno: 2");
//! ```
//!
//! ### Using Macros
//!
//! ```rust,ignore
//! use shared::{bail, ensure};
//! use shared::error::{EngineResult, ErrorCode};
//!
//! fn validate_id(id: i32) -> EngineResult<()> {
//!     ensure!(id > 0, ErrorCode::InvalidArgument, "ID must be positive");
//!     Ok(())
//! }
//!
//! fn must_have_resource() -> EngineResult<String> {
//!     bail!(ErrorCode::NotFound, "required resource missing");
//! }
//! ```
//!
//! ## Error Serialization
//!
//! `EngineError` implements `Serialize`/`Deserialize` for cross-boundary error reporting:
//!
//! ```json
//! {
//!   "code": "NotFound",
//!   "msg": "file not found",
//!   "detail": "/data/missing.txt"
//! }
//! ```

mod codes;

pub use codes::{ErrorCode, io_error_to_error_code};

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Serialize};

/// Structured error type for the Migo engine.
///
/// Contains an error code, human-readable message, and optional debugging details.
/// Designed for efficient construction using builder pattern methods.
///
/// # Examples
///
/// ```rust,ignore
/// use shared::error::{EngineError, ErrorCode};
///
/// // Create with builder pattern
/// let error = EngineError::new(ErrorCode::Timeout)
///     .with_msg("connection timed out")
///     .with_detail("host: example.com, timeout: 30s");
///
/// // Display shows formatted error
/// println!("{}", error); // "[Timeout] connection timed out (host: example.com, timeout: 30s)"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineError {
    /// The error category code.
    pub code: ErrorCode,
    /// Human-readable error message.
    pub msg: Cow<'static, str>,
    /// Optional detailed information for debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Raw OS errno when the error originated from a syscall.
    /// Negative on Unix (standard `errno * -1` convention used by
    /// most JS ecosystems). Absent for pure logic errors.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub errno: Option<i32>,
    /// Target path the operation was acting on, when meaningful
    /// (e.g. `readFile`, `stat`, `unlink`). Absent for errors that
    /// aren't path-bound (network, V8, decoder).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<String>,
    /// The operation name that failed (e.g. "read_file", "stat",
    /// "rename"). Analogous to Node.js' `error.syscall`. Absent
    /// when not applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub op: Option<&'static str>,
}

impl EngineError {
    /// Creates a new error with the given code and its default message.
    ///
    /// # Arguments
    ///
    /// * `code` - The error category code
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let err = EngineError::new(ErrorCode::NotFound);
    /// assert_eq!(err.msg, "not found");
    /// ```
    #[inline]
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            msg: Cow::Borrowed(code.default_message()),
            detail: None,
            errno: None,
            path: None,
            op: None,
        }
    }

    /// Attach the raw OS errno (from `io::Error::raw_os_error`) so
    /// the JS layer can match on POSIX-style codes like `-ENOENT`
    /// without parsing message strings.
    #[inline]
    pub fn with_errno(mut self, errno: i32) -> Self {
        self.errno = Some(errno);
        self
    }

    /// Record the path the failing operation was acting on. Used
    /// by `readFile`, `stat`, `rename`, etc.; skipped when the
    /// error isn't path-bound.
    #[inline]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Record the operation name that failed. Analogous to
    /// `error.syscall` in Node.js (e.g. "open", "stat", "rename").
    #[inline]
    pub fn with_op(mut self, op: &'static str) -> Self {
        self.op = Some(op);
        self
    }

    /// Sets a custom message, replacing the default.
    ///
    /// # Arguments
    ///
    /// * `msg` - Custom error message (supports static str or owned String)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let err = EngineError::new(ErrorCode::InvalidArgument)
    ///     .with_msg("canvas width must be positive");
    /// ```
    #[inline]
    pub fn with_msg(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.msg = msg.into();
        self
    }

    /// Adds detailed debugging information.
    ///
    /// Use this for information that helps diagnose the error but isn't
    /// suitable for the main message (e.g., stack traces, raw values).
    ///
    /// # Arguments
    ///
    /// * `detail` - Additional context for debugging
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let err = EngineError::new(ErrorCode::IoError)
    ///     .with_detail(format!("errno: {}, path: {}", errno, path));
    /// ```
    #[inline]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Creates an error with code and detail in one call.
    ///
    /// Convenience method combining `new()` and `with_detail()`.
    ///
    /// # Arguments
    ///
    /// * `code` - The error category code
    /// * `detail` - Additional context for debugging
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let err = EngineError::from_detail(ErrorCode::NotFound, "user_id: 12345");
    /// ```
    #[inline]
    pub fn from_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self::new(code).with_detail(detail)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(d) if !d.is_empty() => write!(f, "[{:?}] {} ({})", self.code, self.msg, d),
            _ => write!(f, "[{:?}] {}", self.code, self.msg),
        }
    }
}

impl std::error::Error for EngineError {}

pub type EngineResult<T> = Result<T, EngineError>;

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        let code = io_error_to_error_code(&e);
        // Node.js convention: negative errno in the JS surface
        // (`err.errno === -2` for ENOENT, not 2). We mirror that so
        // game code written against the Node/WX `err.errno` shape
        // ports cleanly.
        let errno = e.raw_os_error().map(|n| -n);
        let err = EngineError::new(code).with_detail(e.to_string());
        match errno {
            Some(n) => err.with_errno(n),
            None => err,
        }
    }
}

#[inline]
pub fn code_err(code: ErrorCode) -> EngineError {
    EngineError::new(code)
}

#[inline]
pub fn code_err_detail(code: ErrorCode, detail: impl Into<String>) -> EngineError {
    EngineError::from_detail(code, detail)
}

#[macro_export]
macro_rules! bail {
    ($code:expr) => {
        return Err($crate::error::EngineError::new($code));
    };
    ($code:expr, $msg:expr) => {
        return Err($crate::error::EngineError::new($code).with_msg($msg));
    };
    ($code:expr, $msg:expr, $detail:expr) => {
        return Err($crate::error::EngineError::new($code)
            .with_msg($msg)
            .with_detail($detail));
    };
}

#[macro_export]
macro_rules! ensure {
    ($cond:expr, $code:expr) => {
        if !$cond {
            $crate::bail!($code);
        }
    };
    ($cond:expr, $code:expr, $msg:expr) => {
        if !$cond {
            $crate::bail!($code, $msg);
        }
    };
    ($cond:expr, $code:expr, $msg:expr, $detail:expr) => {
        if !$cond {
            $crate::bail!($code, $msg, $detail);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_error_new() {
        let err = EngineError::new(ErrorCode::NotFound);
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.msg, "not found");
        assert!(err.detail.is_none());
    }

    #[test]
    fn test_engine_error_with_msg() {
        let err = EngineError::new(ErrorCode::Internal).with_msg("custom message");
        assert_eq!(err.code, ErrorCode::Internal);
        assert_eq!(err.msg, "custom message");
    }

    #[test]
    fn test_engine_error_with_detail() {
        let err = EngineError::new(ErrorCode::IoError).with_detail("file not accessible");
        assert_eq!(err.code, ErrorCode::IoError);
        assert_eq!(err.detail, Some("file not accessible".to_string()));
    }

    #[test]
    fn test_engine_error_from_detail() {
        let err = EngineError::from_detail(ErrorCode::Timeout, "connection timed out");
        assert_eq!(err.code, ErrorCode::Timeout);
        assert_eq!(err.detail, Some("connection timed out".to_string()));
    }

    #[test]
    fn test_engine_error_display_without_detail() {
        let err = EngineError::new(ErrorCode::NotFound);
        let display = format!("{}", err);
        assert!(display.contains("NotFound"));
        assert!(display.contains("not found"));
    }

    #[test]
    fn test_engine_error_display_with_detail() {
        let err = EngineError::new(ErrorCode::NotFound).with_detail("user 123");
        let display = format!("{}", err);
        assert!(display.contains("NotFound"));
        assert!(display.contains("not found"));
        assert!(display.contains("user 123"));
    }

    #[test]
    fn test_engine_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: EngineError = io_err.into();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn test_code_err() {
        let err = code_err(ErrorCode::InvalidArgument);
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn test_code_err_detail() {
        let err = code_err_detail(ErrorCode::Unsupported, "feature X not available");
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert_eq!(err.detail, Some("feature X not available".to_string()));
    }

    #[test]
    fn test_engine_error_is_eq() {
        let err1 = EngineError::new(ErrorCode::NotFound);
        let err2 = EngineError::new(ErrorCode::NotFound);
        assert_eq!(err1, err2);
    }

    #[test]
    fn test_engine_error_clone() {
        let err1 = EngineError::new(ErrorCode::Internal).with_detail("cloned");
        let err2 = err1.clone();
        assert_eq!(err1, err2);
    }
}
