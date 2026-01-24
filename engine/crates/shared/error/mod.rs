mod codes;

pub use codes::{ErrorCode, io_error_to_error_code};

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineError {
    pub code: ErrorCode,
    pub msg: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl EngineError {
    #[inline]
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            msg: Cow::Borrowed(code.default_message()),
            detail: None,
        }
    }

    #[inline]
    pub fn with_msg(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.msg = msg.into();
        self
    }

    #[inline]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

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
        EngineError::new(code).with_detail(e.to_string())
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
