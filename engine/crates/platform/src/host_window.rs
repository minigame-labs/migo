//! What content is told about the window it is drawing into.
//!
//! Desktop platforms have no window of their own to measure: the engine layer
//! is handed a surface, and only the embedding host knows what that surface is
//! presented into. Until a host says so, `wx.getSystemInfoSync()` reports a
//! zero-sized window -- and content that lays itself out from
//! `windowWidth`/`windowHeight` stacks everything at the origin while the
//! rendering path stays entirely correct, so no rendering test catches it.
//!
//! Shared by the Linux and Windows platforms because it is the same gap in both
//! and the player drives them through the same code. Android measures its own
//! window through the JVM, and C-ABI hosts have their own version that also
//! handles updates, since a C host can re-attach a resized surface.

use migo_core::services::SystemInfoService;
use shared::protocol::error::ServiceError;
use shared::surface::{SafeArea, WindowInfo};

/// One physical-window measurement supplied by a desktop host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostWindowMetrics {
    width_pixels: u32,
    height_pixels: u32,
    pixel_ratio: f32,
}

impl HostWindowMetrics {
    /// `pixel_ratio` is physical pixels per CSS pixel; a host with no HiDPI
    /// notion passes 1.0.
    ///
    /// Content works in CSS pixels, so this is the number that keeps the two
    /// apart. Reporting physical pixels as CSS pixels lays a game out at the
    /// wrong scale while every pixel still lands exactly where the engine put
    /// it, which is why it reads as a content bug rather than a host one.
    pub fn new(width_pixels: u32, height_pixels: u32, pixel_ratio: f32) -> Self {
        Self {
            width_pixels,
            height_pixels,
            pixel_ratio,
        }
    }
}

/// Reports the window the host described, and nothing else.
///
/// Every other `SystemInfoService` method keeps its default: a desktop platform
/// has no device model, benchmark level or system settings to report, and
/// inventing plausible ones would be worse than saying so.
pub(crate) struct HostWindowInfo {
    metrics: HostWindowMetrics,
}

impl HostWindowInfo {
    pub(crate) fn new(metrics: HostWindowMetrics) -> Self {
        Self { metrics }
    }
}

impl SystemInfoService for HostWindowInfo {
    fn get_window_info_json(&self) -> Result<String, ServiceError> {
        let physical_width = self.metrics.width_pixels as f32;
        let physical_height = self.metrics.height_pixels as f32;
        // A desktop window has no status bar and no display cutout, so the safe
        // area is the whole window -- which is what zero insets mean here.
        let info = WindowInfo {
            pixel_ratio: self.metrics.pixel_ratio,
            screen_width: physical_width,
            screen_height: physical_height,
            window_width: physical_width,
            window_height: physical_height,
            status_bar_height: 0.0,
            screen_top: 0.0,
            safe_area: SafeArea {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            },
        }
        .to_logical();
        serde_json::to_string(&info)
            .map_err(|error| ServiceError::system(format!("getWindowInfo:fail serialize: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_info(metrics: HostWindowMetrics) -> serde_json::Value {
        let json = HostWindowInfo::new(metrics)
            .get_window_info_json()
            .expect("window info must serialise");
        serde_json::from_str(&json).expect("window info must be valid JSON")
    }

    /// The plain case, and the one the Linux player reports: content must see
    /// the window it actually has rather than the zeroes it saw before.
    #[test]
    fn a_host_window_is_reported_to_content() {
        let info = window_info(HostWindowMetrics::new(720, 1280, 1.0));
        assert_eq!(info["window_width"], 720.0);
        assert_eq!(info["window_height"], 1280.0);
        assert_eq!(info["pixel_ratio"], 1.0);
    }

    /// A 2x host presents a 1440x2560 surface and content must be told 720x1280.
    /// Handing it the physical extent is the classic way a game ends up laid out
    /// at the wrong scale.
    #[test]
    fn a_hidpi_window_is_reported_in_css_pixels() {
        let info = window_info(HostWindowMetrics::new(1440, 2560, 2.0));
        assert_eq!(info["window_width"], 720.0);
        assert_eq!(info["window_height"], 1280.0);
        assert_eq!(info["pixel_ratio"], 2.0);
    }

    /// The screen a desktop game is told about is its own window. Reporting the
    /// desktop's extent instead would make a windowed game size itself to the
    /// display.
    #[test]
    fn the_screen_is_the_window() {
        let info = window_info(HostWindowMetrics::new(800, 600, 1.0));
        assert_eq!(info["screen_width"], info["window_width"]);
        assert_eq!(info["screen_height"], info["window_height"]);
    }
}
