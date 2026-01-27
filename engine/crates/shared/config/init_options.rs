//! Initialization options for the Migo engine.

use std::path::{Path, PathBuf};

use deno_core::serde_json::{Map, Value};

/// Extra options for platform-specific or experimental features.
///
/// Stored as a JSON object for easier serialization and cross-language bridging.
/// This allows passing arbitrary key-value pairs from Java/Kotlin to Rust
/// without modifying the core API.
///
/// # Example
///
/// ```rust,ignore
/// use shared::config::Extras;
/// use serde_json::json;
///
/// let mut extras = Extras::new();
/// extras.insert("bluetooth_enabled".into(), json!(true));
/// extras.insert("custom_api_url".into(), json!("https://api.example.com"));
/// ```
pub type Extras = Map<String, Value>;

/// Log level for the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum LogLevel {
    None = 0,
    Error = 1,
    #[default]
    Warn = 2,
    Info = 3,
    Debug = 4,
    Verbose = 5,
}

impl From<i32> for LogLevel {
    fn from(value: i32) -> Self {
        match value {
            0 => LogLevel::None,
            1 => LogLevel::Error,
            2 => LogLevel::Warn,
            3 => LogLevel::Info,
            4 => LogLevel::Debug,
            5 => LogLevel::Verbose,
            _ => LogLevel::Warn,
        }
    }
}

/// Configuration options for initializing the Migo engine.
///
/// These options are typically provided by the platform layer (Java/Android)
/// and passed through JNI when creating a new host instance.
///
/// # Builder Pattern
///
/// Use the builder-style methods to construct options:
///
/// ```rust,ignore
/// use shared::config::InitOptions;
///
/// let options = InitOptions::new()
///     .with_pixel_ratio(2.0)
///     .with_tmp_dir("/data/app/cache".into())
///     .with_extra("feature_flag", true);
/// ```
///
/// # Platform Integration
///
/// On Android, these options are constructed from `InitOption.java`:
///
/// ```java
/// InitOption option = new InitOption.Builder(context)
///     .setFullScreen(true)
///     .setTargetFps(60)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Device pixel ratio (DPI scale factor).
    /// Values > 1.0 indicate high-DPI displays (e.g., 2.0 = Retina, 3.0 = xxhdpi).
    pixel_ratio: f32,
    /// Temporary directory for cache files, decoded audio, etc.
    tmp_dir: PathBuf,
    /// Code cache directory for compiled scripts.
    code_cache_dir: PathBuf,
    /// Target frames per second (1-120).
    target_fps: i32,
    /// Whether debug mode is enabled.
    debug_enabled: bool,
    /// Log level for the engine.
    log_level: LogLevel,
    /// Maximum memory limit for JavaScript runtime in MB.
    max_memory_mb: i32,
    /// Platform-specific or experimental options.
    extras: Extras,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            pixel_ratio: 1.0,
            tmp_dir: std::env::temp_dir(),
            code_cache_dir: std::env::temp_dir(),
            target_fps: 60,
            debug_enabled: false,
            log_level: LogLevel::Warn,
            max_memory_mb: 512,
            extras: Extras::new(),
        }
    }
}

impl InitOptions {
    /// Creates a new `InitOptions` with default values.
    ///
    /// Defaults:
    /// - `pixel_ratio`: 1.0
    /// - `tmp_dir`: System temp directory
    /// - `target_fps`: 60
    /// - `debug_enabled`: false
    /// - `log_level`: Warn
    /// - `max_memory_mb`: 512
    /// - `extras`: Empty map
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the device pixel ratio.
    ///
    /// This value is used to convert between logical (CSS) pixels and
    /// physical (device) pixels. A value of 2.0 means 1 CSS pixel = 2 device pixels.
    #[inline]
    pub fn pixel_ratio(&self) -> f32 {
        self.pixel_ratio
    }

    /// Returns the temporary directory path.
    ///
    /// Used for caching decoded audio, intermediate files, etc.
    #[inline]
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    /// Returns the code cache directory path.
    ///
    /// Used for caching compiled JavaScript bytecode.
    #[inline]
    pub fn code_cache_dir(&self) -> &Path {
        &self.code_cache_dir
    }

    /// Returns the target frames per second.
    #[inline]
    pub fn target_fps(&self) -> i32 {
        self.target_fps
    }

    /// Returns whether debug mode is enabled.
    #[inline]
    pub fn debug_enabled(&self) -> bool {
        self.debug_enabled
    }

    /// Returns the log level.
    #[inline]
    pub fn log_level(&self) -> LogLevel {
        self.log_level
    }

    /// Returns the maximum memory limit in MB.
    #[inline]
    pub fn max_memory_mb(&self) -> i32 {
        self.max_memory_mb
    }

    /// Returns a reference to the extras map.
    #[inline]
    pub fn extras(&self) -> &Extras {
        &self.extras
    }

    /// Returns a mutable reference to the extras map.
    #[inline]
    pub fn extras_mut(&mut self) -> &mut Extras {
        &mut self.extras
    }

    /// Sets the pixel ratio (builder pattern).
    ///
    /// Invalid values (NaN, Inf, ≤0) are silently replaced with 1.0.
    ///
    /// # Arguments
    ///
    /// * `pixel_ratio` - Device pixel ratio (typically 1.0–4.0)
    #[must_use]
    pub fn with_pixel_ratio(mut self, pixel_ratio: f32) -> Self {
        // Defensive: avoid NaN/Inf/<=0 from JNI/JS inputs.
        self.pixel_ratio = if pixel_ratio.is_finite() && pixel_ratio > 0.0 {
            pixel_ratio
        } else {
            1.0
        };
        self
    }

    /// Sets the temporary directory (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `tmp_dir` - Path to the temporary directory
    #[must_use]
    pub fn with_tmp_dir(mut self, tmp_dir: PathBuf) -> Self {
        self.tmp_dir = tmp_dir;
        self
    }

    /// Sets the code cache directory (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `code_cache_dir` - Path to the code cache directory
    #[must_use]
    pub fn with_code_cache_dir(mut self, code_cache_dir: PathBuf) -> Self {
        self.code_cache_dir = code_cache_dir;
        self
    }

    /// Sets the target FPS (builder pattern).
    ///
    /// Values are clamped to the range [1, 120].
    ///
    /// # Arguments
    ///
    /// * `fps` - Target frames per second
    #[must_use]
    pub fn with_target_fps(mut self, fps: i32) -> Self {
        self.target_fps = fps.clamp(1, 120);
        self
    }

    /// Sets debug mode (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether debug mode should be enabled
    #[must_use]
    pub fn with_debug_enabled(mut self, enabled: bool) -> Self {
        self.debug_enabled = enabled;
        self
    }

    /// Sets the log level (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `level` - Log level
    #[must_use]
    pub fn with_log_level(mut self, level: LogLevel) -> Self {
        self.log_level = level;
        self
    }

    /// Sets the maximum memory limit (builder pattern).
    ///
    /// Values are clamped to the range [64, 2048].
    ///
    /// # Arguments
    ///
    /// * `mb` - Maximum memory in megabytes
    #[must_use]
    pub fn with_max_memory_mb(mut self, mb: i32) -> Self {
        self.max_memory_mb = mb.clamp(64, 2048);
        self
    }

    /// Adds a key-value pair to the extras map (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `k` - Key name
    /// * `v` - Value (any type that converts to JSON Value)
    #[must_use]
    pub fn with_extra<V: Into<Value>>(mut self, k: impl Into<String>, v: V) -> Self {
        self.extras.insert(k.into(), v.into());
        self
    }
}
