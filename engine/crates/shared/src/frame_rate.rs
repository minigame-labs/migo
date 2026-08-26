//! The one rule for what frame rate content may ask for.
//!
//! Five places need it -- the JS entry point, the render command, the vsync
//! decimator, the engine-paced clock and the host's startup option -- and until
//! they shared this module each spelled its own `clamp(1, 120)`. Two spellings of
//! one rule is the defect, not the design: a range widened in four of five places
//! is a range that silently keeps the old ceiling wherever it was missed.

/// Below one frame per second there is no animation to pace, and the interval
/// arithmetic (`1000 / fps`) needs a non-zero divisor.
pub const MIN_FPS: u32 = 1;

/// The fastest panel the engine serves. Not a taste limit: on platforms with no
/// vsync callback the engine paces itself from this number, so a request past
/// any real display's rate is work no one can see.
pub const MAX_FPS: u32 = 240;

/// What content gets before anyone asks. The engine-paced clock, the vsync
/// decimator, the host's startup option and the window's frame-rate request all
/// start here, and they have to start at the same place: a window asked for one
/// rate while the clock paces another is the mismatch this module exists to
/// remove.
pub const DEFAULT_FPS: u32 = 60;

/// The requested rate the engine will actually serve.
#[inline]
#[must_use]
pub fn clamp_fps(fps: u32) -> u32 {
    fps.clamp(MIN_FPS, MAX_FPS)
}

/// The requested rate from a JavaScript Number.
///
/// `None` means the argument was not a rate at all, and the current one stands.
/// Rounding rather than truncating because a caller asking for a panel's real
/// rate (59.94) means the nearest rate the engine can serve, and truncation used
/// to answer 59 -- a rate no panel runs at, which then paced against a 60Hz grid.
#[inline]
#[must_use]
pub fn requested_fps(fps: f64) -> Option<u32> {
    if !fps.is_finite() {
        return None;
    }
    Some(fps.round().clamp(MIN_FPS as f64, MAX_FPS as f64) as u32)
}

#[cfg(test)]
mod tests {
    use super::{MAX_FPS, MIN_FPS, clamp_fps, requested_fps};

    #[test]
    fn the_range_is_closed_at_both_ends() {
        assert_eq!(clamp_fps(0), MIN_FPS);
        assert_eq!(clamp_fps(u32::MAX), MAX_FPS);
        assert_eq!(clamp_fps(60), 60);
    }

    /// The two requests that a JS-side `fps | 0` used to turn into 1 fps -- the
    /// bottom of the range -- while meaning anything but that.
    #[test]
    fn a_request_that_is_not_a_rate_leaves_the_current_rate_alone() {
        assert_eq!(requested_fps(f64::NAN), None, "NaN is not a frame rate");
        assert_eq!(requested_fps(f64::INFINITY), None);
        assert_eq!(
            requested_fps(3e9),
            Some(MAX_FPS),
            "a request past the range is the top of it, not the bottom"
        );
    }

    #[test]
    fn a_fractional_request_takes_the_nearest_servable_rate() {
        assert_eq!(
            requested_fps(59.94),
            Some(60),
            "asking for a panel's real rate must not truncate to 59"
        );
        assert_eq!(requested_fps(0.4), Some(MIN_FPS));
        assert_eq!(requested_fps(-30.0), Some(MIN_FPS));
        assert_eq!(requested_fps(240.0), Some(MAX_FPS));
    }
}
