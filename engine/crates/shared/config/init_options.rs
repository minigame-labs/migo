use std::path::{Path, PathBuf};

use deno_core::serde_json::{Map, Value};

/// Extra options for platform-specific or experimental features.
/// Stored as a JSON object for easier serialization and cross-language bridging.
pub type Extras = Map<String, Value>;

#[derive(Debug, Clone)]
pub struct InitOptions {
    pixel_ratio: f32,
    tmp_dir: PathBuf,
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
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn pixel_ratio(&self) -> f32 {
        self.pixel_ratio
    }

    #[inline]
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    #[inline]
    pub fn extras(&self) -> &Extras {
        &self.extras
    }

    #[inline]
    pub fn extras_mut(&mut self) -> &mut Extras {
        &mut self.extras
    }

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

    #[must_use]
    pub fn with_tmp_dir(mut self, tmp_dir: PathBuf) -> Self {
        self.tmp_dir = tmp_dir;
        self
    }

    #[must_use]
    pub fn with_extra<V: Into<Value>>(mut self, k: impl Into<String>, v: V) -> Self {
        self.extras.insert(k.into(), v.into());
        self
    }
}
