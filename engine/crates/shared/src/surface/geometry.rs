use serde::{Deserialize, Serialize};

/// A finite, strictly-positive device-pixel ratio.
///
/// Keeping this invariant in the type prevents an invalid scale from entering
/// the asynchronous Host/Render command stream and failing much later on the
/// render thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRatio(f32);

impl PixelRatio {
    #[inline]
    pub fn new(value: f32) -> Option<Self> {
        (value.is_finite() && value > 0.0).then_some(Self(value))
    }

    #[inline]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Stable reason categories for an unexpected live-Surface retirement.
///
/// Values intentionally match the public C ABI. `HostDestroyed` describes an
/// expected host action and is never emitted as an unexpected-loss callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SurfaceLossReason {
    Unknown = 0,
    HostDestroyed = 1,
    DeviceLost = 2,
    PlatformError = 3,
}

impl SurfaceLossReason {
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct SafeArea {
    /// Insets from the corresponding physical/logical window edges.
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SafeArea {
    #[inline]
    pub fn is_zero(self) -> bool {
        self.left == 0.0 && self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.left.is_finite()
            && self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct WindowInfo {
    pub pixel_ratio: f32,
    pub screen_width: f32,
    pub screen_height: f32,
    pub window_width: f32,
    pub window_height: f32,
    pub status_bar_height: f32,
    pub screen_top: f32,
    pub safe_area: SafeArea,
}

impl WindowInfo {
    #[inline]
    pub fn is_valid(self) -> bool {
        self.pixel_ratio.is_finite()
            && self.pixel_ratio > 0.0
            && self.screen_width.is_finite()
            && self.screen_height.is_finite()
            && self.window_width.is_finite()
            && self.window_height.is_finite()
            && self.status_bar_height.is_finite()
            && self.screen_top.is_finite()
            && self.safe_area.is_finite()
    }

    /// Convert physical pixels to logical pixels using `pixel_ratio`.
    #[inline]
    pub fn to_logical(self) -> Self {
        if !self.pixel_ratio.is_finite()
            || self.pixel_ratio <= 0.0
            || (self.pixel_ratio - 1.0).abs() <= f32::EPSILON
        {
            return self;
        }

        let pr = self.pixel_ratio;
        Self {
            pixel_ratio: pr,
            screen_width: self.screen_width / pr,
            screen_height: self.screen_height / pr,
            window_width: self.window_width / pr,
            window_height: self.window_height / pr,
            status_bar_height: self.status_bar_height / pr,
            screen_top: self.screen_top / pr,
            safe_area: SafeArea {
                left: self.safe_area.left / pr,
                top: self.safe_area.top / pr,
                right: self.safe_area.right / pr,
                bottom: self.safe_area.bottom / pr,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PixelRatio, SurfaceLossReason};

    #[test]
    fn pixel_ratio_accepts_only_finite_positive_values() {
        assert_eq!(PixelRatio::new(2.5).map(PixelRatio::get), Some(2.5));
        assert!(PixelRatio::new(0.0).is_none());
        assert!(PixelRatio::new(-1.0).is_none());
        assert!(PixelRatio::new(f32::NAN).is_none());
        assert!(PixelRatio::new(f32::INFINITY).is_none());
    }

    #[test]
    fn surface_loss_reason_values_are_stable_for_native_abis() {
        assert_eq!(SurfaceLossReason::Unknown.as_u32(), 0);
        assert_eq!(SurfaceLossReason::HostDestroyed.as_u32(), 1);
        assert_eq!(SurfaceLossReason::DeviceLost.as_u32(), 2);
        assert_eq!(SurfaceLossReason::PlatformError.as_u32(), 3);
    }
}
