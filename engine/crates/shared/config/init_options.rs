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
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Device pixel ratio (DPI scale factor).
    /// Values > 1.0 indicate high-DPI displays (e.g., 2.0 = Retina, 3.0 = xxhdpi).
    pixel_ratio: f32,
    /// Temporary directory for cache files, decoded audio, etc.
    tmp_dir: PathBuf,
    /// Platform-specific or experimental options.
    extras: Extras,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            pixel_ratio: 1.0,
            tmp_dir: std::env::temp_dir(),
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
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let options = InitOptions::new().with_pixel_ratio(2.5);
    /// assert_eq!(options.pixel_ratio(), 2.5);
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let options = InitOptions::new()
    ///     .with_tmp_dir("/data/user/0/com.app/cache".into());
    /// ```
    #[must_use]
    pub fn with_tmp_dir(mut self, tmp_dir: PathBuf) -> Self {
        self.tmp_dir = tmp_dir;
        self
    }

    /// Adds a key-value pair to the extras map (builder pattern).
    ///
    /// # Arguments
    ///
    /// * `k` - Key name
    /// * `v` - Value (any type that converts to JSON Value)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let options = InitOptions::new()
    ///     .with_extra("debug_mode", true)
    ///     .with_extra("log_level", "verbose")
    ///     .with_extra("max_fps", 120);
    /// ```
    #[must_use]
    pub fn with_extra<V: Into<Value>>(mut self, k: impl Into<String>, v: V) -> Self {
        self.extras.insert(k.into(), v.into());
        self
    }
}
