use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    Ok = 0,

    // General (1..99)
    Internal = 1,
    InvalidArgument = 2,
    NotFound = 3,
    PermissionDenied = 4,
    Timeout = 5,
    Unsupported = 6,
    NotImplemented = 7,
    Cancelled = 8,

    /// Cross-thread / channel disconnected, sender dropped, etc.
    Disconnected = 9,
    InvalidOperation = 10,

    // IO / FS (100..199)
    IoError = 100,
    BadFileDescriptor = 101,
    ExceedMaxConcurrentFdLimit = 102,

    // V8 / JS / Buffer (200..299)
    ArrayBufferDoesNotExist = 200,
    JsException = 201,
    ModuleLoadError = 202,

    // Image (300..399)
    ImageReadError = 300,
    InvalidImageBuffer = 301,

    // Render / GL (legacy/general) (400..402)
    RenderBackendError = 400,
    ShaderCompileError = 401,
    ProgramLinkError = 402,

    // Render backend / platform / context / surface (generic, backend-agnostic) (403..419)
    RenderLibraryLoadError = 403,
    RenderSymbolLoadError = 404,
    RenderBindApiError = 405,
    RenderGetDisplayError = 406,
    RenderInitializeError = 407,
    RenderChooseConfigError = 408,
    RenderCreateSurfaceError = 409,
    RenderCreateContextError = 410,
    RenderMakeCurrentError = 411,
    RenderSwapIntervalError = 412,
    RenderSwapBuffersError = 413,
    RenderInvalidStateError = 414,

    // 2D / Canvas / Vector-graphics subsystem (generic) (420..429)
    Render2DInitError = 420,
    Render2DResourceError = 421,
}

impl ErrorCode {
    #[inline]
    pub const fn default_message(self) -> &'static str {
        match self {
            ErrorCode::Ok => "ok",

            ErrorCode::Internal => "internal error",
            ErrorCode::InvalidArgument => "invalid argument",
            ErrorCode::NotFound => "not found",
            ErrorCode::PermissionDenied => "permission denied",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::NotImplemented => "not implemented",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::Disconnected => "disconnected",
            ErrorCode::InvalidOperation => "invalid operation",

            ErrorCode::IoError => "io error",
            ErrorCode::BadFileDescriptor => "bad file descriptor",
            ErrorCode::ExceedMaxConcurrentFdLimit => "too many open files",

            ErrorCode::ArrayBufferDoesNotExist => "array buffer does not exist",
            ErrorCode::JsException => "js exception",
            ErrorCode::ModuleLoadError => "module load error",

            ErrorCode::ImageReadError => "image read error",
            ErrorCode::InvalidImageBuffer => "invalid image buffer",

            ErrorCode::RenderBackendError => "render backend error",
            ErrorCode::ShaderCompileError => "shader compile error",
            ErrorCode::ProgramLinkError => "program link error",

            ErrorCode::RenderLibraryLoadError => "render library load error",
            ErrorCode::RenderSymbolLoadError => "render symbol load error",
            ErrorCode::RenderBindApiError => "render bind api error",
            ErrorCode::RenderGetDisplayError => "render get display error",
            ErrorCode::RenderInitializeError => "render initialize error",
            ErrorCode::RenderChooseConfigError => "render choose config error",
            ErrorCode::RenderCreateSurfaceError => "render create surface error",
            ErrorCode::RenderCreateContextError => "render create context error",
            ErrorCode::RenderMakeCurrentError => "render make current error",
            ErrorCode::RenderSwapIntervalError => "render swap interval error",
            ErrorCode::RenderSwapBuffersError => "render swap buffers error",
            ErrorCode::RenderInvalidStateError => "render invalid state error",

            ErrorCode::Render2DInitError => "render 2d init error",
            ErrorCode::Render2DResourceError => "render 2d resource error",
        }
    }

    #[inline]
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

impl From<ErrorCode> for u16 {
    #[inline]
    fn from(code: ErrorCode) -> Self {
        code.as_u16()
    }
}

impl TryFrom<u16> for ErrorCode {
    type Error = ();

    #[inline]
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        let code = match v {
            0 => ErrorCode::Ok,

            1 => ErrorCode::Internal,
            2 => ErrorCode::InvalidArgument,
            3 => ErrorCode::NotFound,
            4 => ErrorCode::PermissionDenied,
            5 => ErrorCode::Timeout,
            6 => ErrorCode::Unsupported,
            7 => ErrorCode::NotImplemented,
            8 => ErrorCode::Cancelled,
            9 => ErrorCode::Disconnected,
            10 => ErrorCode::InvalidOperation,

            100 => ErrorCode::IoError,
            101 => ErrorCode::BadFileDescriptor,
            102 => ErrorCode::ExceedMaxConcurrentFdLimit,

            200 => ErrorCode::ArrayBufferDoesNotExist,
            201 => ErrorCode::JsException,
            202 => ErrorCode::ModuleLoadError,

            300 => ErrorCode::ImageReadError,
            301 => ErrorCode::InvalidImageBuffer,

            400 => ErrorCode::RenderBackendError,
            401 => ErrorCode::ShaderCompileError,
            402 => ErrorCode::ProgramLinkError,

            403 => ErrorCode::RenderLibraryLoadError,
            404 => ErrorCode::RenderSymbolLoadError,
            405 => ErrorCode::RenderBindApiError,
            406 => ErrorCode::RenderGetDisplayError,
            407 => ErrorCode::RenderInitializeError,
            408 => ErrorCode::RenderChooseConfigError,
            409 => ErrorCode::RenderCreateSurfaceError,
            410 => ErrorCode::RenderCreateContextError,
            411 => ErrorCode::RenderMakeCurrentError,
            412 => ErrorCode::RenderSwapIntervalError,
            413 => ErrorCode::RenderSwapBuffersError,
            414 => ErrorCode::RenderInvalidStateError,

            420 => ErrorCode::Render2DInitError,
            421 => ErrorCode::Render2DResourceError,

            _ => return Err(()),
        };
        Ok(code)
    }
}

/// std::io::ErrorKind is Copy, so `ErrorCode::from(e.kind())` is correct.
impl From<std::io::ErrorKind> for ErrorCode {
    #[inline]
    fn from(kind: std::io::ErrorKind) -> Self {
        use std::io::ErrorKind::*;
        match kind {
            NotFound => ErrorCode::NotFound,
            PermissionDenied => ErrorCode::PermissionDenied,
            TimedOut => ErrorCode::Timeout,
            Unsupported => ErrorCode::Unsupported,
            InvalidInput | InvalidData => ErrorCode::InvalidArgument,
            _ => ErrorCode::IoError,
        }
    }
}

#[inline]
pub fn io_error_to_error_code(e: &std::io::Error) -> ErrorCode {
    ErrorCode::from(e.kind())
}
