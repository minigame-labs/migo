//! Pure frame-rate hint policy for `ANativeWindow_setFrameRate`.
//!
//! This module deliberately has no platform dependency so host tests can prove
//! which compatibility value a game asks for before an Android build runs.
//! `platform/src/android/**` is compiled only on Android, so anything decided
//! inside `surface.rs` is never *executed* by a test -- only compiled. The
//! decision therefore lives here and `surface.rs` is a caller.

/// The `ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_*` values, which are ABI
/// constants of `libandroid.so` rather than something we choose the numbers of.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameRateCompatibility {
    /// The content has no inherent fixed frame rate.
    Default = 0,
    /// The content is presenting at a fixed rate that pull-down would disturb.
    ///
    /// Never passed. It is named so the value we decline is written down next to
    /// the one we pass, and so the ABI test below can assert both numbers -- a
    /// lone constant proves nothing about the pair it has to be distinct from.
    #[allow(dead_code)]
    FixedSource = 1,
}

impl FrameRateCompatibility {
    pub(crate) const fn as_abi(self) -> i8 {
        self as i8
    }
}

/// The compatibility a game's surface asks for.
///
/// `DEFAULT`, not `FIXED_SOURCE`. The NDK documents the split by what the
/// content *is*, not by whether it wants a steady rate: `FIXED_SOURCE` says the
/// content has an inherently fixed rate that the app cannot adapt away from, so
/// a system rate other than the requested one forces pull-down and a likely
/// stuttery result -- that is video. `DEFAULT` says the app can simply run at
/// whatever rate the system picks, and the NDK states outright that it "should
/// be used when displaying game content, UIs, and anything that isn't video".
///
/// A game asking for 60 fps is not fixed-rate content; it is content that would
/// *like* 60 and can render at whatever it gets. Declaring it `FIXED_SOURCE`
/// tells SurfaceFlinger to weigh a mode switch against a pull-down cost this
/// content never pays, which is the wrong input to the decision -- and on a
/// panel where the switch is declined it claims a stutter that is not there.
pub(crate) const fn game_compatibility() -> FrameRateCompatibility {
    FrameRateCompatibility::Default
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_asks_for_default_compatibility() {
        assert_eq!(game_compatibility(), FrameRateCompatibility::Default);
    }

    #[test]
    fn the_abi_values_match_libandroid() {
        assert_eq!(FrameRateCompatibility::Default.as_abi(), 0);
        assert_eq!(FrameRateCompatibility::FixedSource.as_abi(), 1);
    }
}
