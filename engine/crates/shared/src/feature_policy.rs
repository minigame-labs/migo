//! Central decision point for optional engine features.
//!
//! Every performance feature that can be wrong on some device needs the same
//! four things: a stable name, a default, a way to turn it off without shipping
//! a build, and a recorded reason when it did not run. Spreading those across
//! call sites is how a fleet ends up in a state nobody can name -- and how a
//! low-tier fallback path stops being executed without anything going red.
//!
//! The decision order is fixed and total. Each layer can only ever *disable*,
//! so a feature that survives to the end ran for a reason that can be stated,
//! and one that did not carries the layer that stopped it:
//!
//! 1. build support -- the code is not compiled in
//! 2. local override -- an operator or a test said no
//! 3. capability -- the device lacks the extension/API it needs
//! 4. driver denylist -- this GPU/driver is known to get it wrong
//! 5. remote policy -- a kill switch was thrown for this build
//! 6. initialisation -- the resource it needs could not be created
//! 7. health check -- it ran and misbehaved
//!
//! This module is pure: no environment, no clock, no I/O. `from_env` builds the
//! override map once at start-up and hands it in.

use std::collections::BTreeMap;

/// A feature whose state has to be decidable and reportable.
///
/// The string form is the stable name: it appears in telemetry, in override
/// strings, and in bug reports, so it outlives any rename of the Rust variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum FeatureKey {
    /// Paint simple `fillText` runs through a cached `SkTextBlob` instead of
    /// building an `SkParagraph` per call.
    CanvasTextBlobFastPath,
    /// Merge eligible consecutive `drawImage` runs into one `SkCanvas::drawAtlas`.
    CanvasDrawAtlas,
    /// Bound how many cold offscreen canvases keep a GPU backing.
    CanvasHotBackingPool,
    /// Submit surface damage to the compositor via `eglSwapBuffersWithDamage`.
    PresentSwapDamage,
    /// Drive presentation through the Android Frame Pacing library.
    PresentSwappy,
    /// Poll shader completion without blocking via `KHR_parallel_shader_compile`.
    WebglParallelShaderCompile,
    /// Select ASTC texture variants where the device supports them.
    TextureAstcVariants,
    /// Publish a thermal-derived quality-pressure advisory to the host.
    RuntimeQualityPressure,
}

impl FeatureKey {
    /// Every key, so exhaustive reporting cannot drift from the enum.
    pub const ALL: [Self; 8] = [
        Self::CanvasTextBlobFastPath,
        Self::CanvasDrawAtlas,
        Self::CanvasHotBackingPool,
        Self::PresentSwapDamage,
        Self::PresentSwappy,
        Self::WebglParallelShaderCompile,
        Self::TextureAstcVariants,
        Self::RuntimeQualityPressure,
    ];

    /// The stable name. Changing one of these is a protocol change.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CanvasTextBlobFastPath => "canvas_text_blob_fast_path",
            Self::CanvasDrawAtlas => "canvas_draw_atlas",
            Self::CanvasHotBackingPool => "canvas_hot_backing_pool",
            Self::PresentSwapDamage => "present_swap_damage",
            Self::PresentSwappy => "present_swappy",
            Self::WebglParallelShaderCompile => "webgl_parallel_shader_compile",
            Self::TextureAstcVariants => "texture_astc_variants",
            Self::RuntimeQualityPressure => "runtime_quality_pressure",
        }
    }

    /// Parse a stable name back to a key.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.name() == name)
    }

    /// Whether this feature runs unless something turns it off.
    ///
    /// A feature is only defaulted on once it has cleared its release gate on
    /// the device matrix; until then the honest default is off, so that a build
    /// nobody measured behaves like the build that was measured.
    pub const fn default_enabled(self) -> bool {
        match self {
            // Proven by host pixel-parity goldens across the font/size/
            // alignment/baseline matrix, with every ineligible case falling
            // back to SkParagraph.
            Self::CanvasTextBlobFastPath => true,
            // Surface damage is a compositor hint with a plain-swap fallback on
            // the first rejection, and it only ever describes a region that was
            // actually redrawn.
            Self::PresentSwapDamage => true,
            Self::CanvasDrawAtlas
            | Self::CanvasHotBackingPool
            | Self::PresentSwappy
            | Self::WebglParallelShaderCompile
            | Self::TextureAstcVariants
            | Self::RuntimeQualityPressure => false,
        }
    }
}

/// Why a feature is not running.
///
/// Ordered by the layer that produced it, so a report can be read as "how far
/// did this get".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackReason {
    /// Not compiled into this build.
    BuildUnsupported,
    /// An operator, a test, or a local config turned it off.
    LocalOverride,
    /// Off by default and nothing turned it on.
    DefaultOff,
    /// The device lacks a capability it requires.
    CapabilityMissing,
    /// This GPU/driver is known to get it wrong.
    DriverDenylisted,
    /// A remote kill switch is set for this build.
    RemotePolicy,
    /// It could not initialise the resources it needs.
    InitFailed,
    /// It initialised, ran, and then misbehaved.
    HealthCheckFailed,
}

impl FallbackReason {
    /// The stable name, for telemetry.
    pub const fn name(self) -> &'static str {
        match self {
            Self::BuildUnsupported => "build_unsupported",
            Self::LocalOverride => "local_override",
            Self::DefaultOff => "default_off",
            Self::CapabilityMissing => "capability_missing",
            Self::DriverDenylisted => "driver_denylisted",
            Self::RemotePolicy => "remote_policy",
            Self::InitFailed => "init_failed",
            Self::HealthCheckFailed => "health_check_failed",
        }
    }
}

/// The outcome for one feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureDecision {
    pub key: FeatureKey,
    pub enabled: bool,
    /// `None` exactly when `enabled` is true.
    pub reason: Option<FallbackReason>,
}

/// What each layer says about a feature. Absent entries mean "no objection".
#[derive(Clone, Debug, Default)]
pub struct FeaturePolicy {
    build_unsupported: BTreeMap<FeatureKey, ()>,
    local_override: BTreeMap<FeatureKey, bool>,
    capability_missing: BTreeMap<FeatureKey, ()>,
    driver_denylisted: BTreeMap<FeatureKey, ()>,
    remote_disabled: BTreeMap<FeatureKey, ()>,
    init_failed: BTreeMap<FeatureKey, ()>,
    health_failed: BTreeMap<FeatureKey, ()>,
}

impl FeaturePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a feature as absent from this build.
    pub fn set_build_unsupported(&mut self, key: FeatureKey) -> &mut Self {
        self.build_unsupported.insert(key, ());
        self
    }

    /// Force a feature on or off locally. `false` always wins; `true` only
    /// overrides the default, never a later layer -- an operator can ask for a
    /// feature, but cannot ask a device to have a capability it lacks.
    pub fn set_local_override(&mut self, key: FeatureKey, enabled: bool) -> &mut Self {
        self.local_override.insert(key, enabled);
        self
    }

    /// Mark a required device capability as missing.
    pub fn set_capability_missing(&mut self, key: FeatureKey) -> &mut Self {
        self.capability_missing.insert(key, ());
        self
    }

    /// Mark this GPU/driver as known-bad for the feature.
    pub fn set_driver_denylisted(&mut self, key: FeatureKey) -> &mut Self {
        self.driver_denylisted.insert(key, ());
        self
    }

    /// Throw the remote kill switch.
    pub fn set_remote_disabled(&mut self, key: FeatureKey) -> &mut Self {
        self.remote_disabled.insert(key, ());
        self
    }

    /// Record that the feature could not initialise.
    pub fn set_init_failed(&mut self, key: FeatureKey) -> &mut Self {
        self.init_failed.insert(key, ());
        self
    }

    /// Record that the feature misbehaved after starting.
    pub fn set_health_failed(&mut self, key: FeatureKey) -> &mut Self {
        self.health_failed.insert(key, ());
        self
    }

    /// Resolve one feature. Total, deterministic, and free of side effects.
    pub fn decide(&self, key: FeatureKey) -> FeatureDecision {
        let deny = |reason: FallbackReason| FeatureDecision {
            key,
            enabled: false,
            reason: Some(reason),
        };

        if self.build_unsupported.contains_key(&key) {
            return deny(FallbackReason::BuildUnsupported);
        }
        let wanted = match self.local_override.get(&key) {
            Some(false) => return deny(FallbackReason::LocalOverride),
            Some(true) => true,
            None => key.default_enabled(),
        };
        if !wanted {
            return deny(FallbackReason::DefaultOff);
        }
        if self.capability_missing.contains_key(&key) {
            return deny(FallbackReason::CapabilityMissing);
        }
        if self.driver_denylisted.contains_key(&key) {
            return deny(FallbackReason::DriverDenylisted);
        }
        if self.remote_disabled.contains_key(&key) {
            return deny(FallbackReason::RemotePolicy);
        }
        if self.init_failed.contains_key(&key) {
            return deny(FallbackReason::InitFailed);
        }
        if self.health_failed.contains_key(&key) {
            return deny(FallbackReason::HealthCheckFailed);
        }
        FeatureDecision {
            key,
            enabled: true,
            reason: None,
        }
    }

    /// Convenience for hot-path owners that resolve once and cache a bool.
    pub fn is_enabled(&self, key: FeatureKey) -> bool {
        self.decide(key).enabled
    }

    /// Every feature's decision, in key order, for a telemetry record.
    pub fn decisions(&self) -> Vec<FeatureDecision> {
        FeatureKey::ALL
            .into_iter()
            .map(|k| self.decide(k))
            .collect()
    }

    /// Apply overrides from a `name=on,name2=off` list.
    ///
    /// Unknown names are reported rather than ignored: a kill switch that
    /// silently does nothing because of a typo is worse than no kill switch,
    /// because it is believed.
    pub fn apply_override_list(&mut self, list: &str) -> Result<(), String> {
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (name, value) = item
                .split_once('=')
                .ok_or_else(|| format!("feature override {item:?} is not name=on|off"))?;
            let key = FeatureKey::from_name(name.trim())
                .ok_or_else(|| format!("unknown feature {:?}", name.trim()))?;
            let enabled = match value.trim() {
                "on" | "1" | "true" => true,
                "off" | "0" | "false" => false,
                other => return Err(format!("feature {name:?} value {other:?} is not on|off")),
            };
            self.set_local_override(key, enabled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_round_trip() {
        for key in FeatureKey::ALL {
            assert_eq!(FeatureKey::from_name(key.name()), Some(key));
        }
        let mut names: Vec<&str> = FeatureKey::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two features share a stable name");
    }

    #[test]
    fn a_feature_that_runs_has_no_reason_and_one_that_does_not_always_has_one() {
        let policy = FeaturePolicy::new();
        for decision in policy.decisions() {
            assert_eq!(
                decision.enabled,
                decision.reason.is_none(),
                "{} reported enabled={} with reason={:?}",
                decision.key.name(),
                decision.enabled,
                decision.reason
            );
        }
    }

    #[test]
    fn each_layer_denies_with_its_own_reason() {
        let key = FeatureKey::CanvasTextBlobFastPath;
        assert!(
            key.default_enabled(),
            "this test needs a default-on feature"
        );

        let cases: [(fn(&mut FeaturePolicy, FeatureKey), FallbackReason); 6] = [
            (
                |p, k| {
                    p.set_build_unsupported(k);
                },
                FallbackReason::BuildUnsupported,
            ),
            (
                |p, k| {
                    p.set_capability_missing(k);
                },
                FallbackReason::CapabilityMissing,
            ),
            (
                |p, k| {
                    p.set_driver_denylisted(k);
                },
                FallbackReason::DriverDenylisted,
            ),
            (
                |p, k| {
                    p.set_remote_disabled(k);
                },
                FallbackReason::RemotePolicy,
            ),
            (
                |p, k| {
                    p.set_init_failed(k);
                },
                FallbackReason::InitFailed,
            ),
            (
                |p, k| {
                    p.set_health_failed(k);
                },
                FallbackReason::HealthCheckFailed,
            ),
        ];

        for (apply, expected) in cases {
            let mut policy = FeaturePolicy::new();
            apply(&mut policy, key);
            let decision = policy.decide(key);
            assert!(!decision.enabled);
            assert_eq!(decision.reason, Some(expected));
        }
    }

    #[test]
    fn an_operator_can_ask_for_a_feature_but_not_for_a_capability() {
        let key = FeatureKey::CanvasDrawAtlas;
        assert!(
            !key.default_enabled(),
            "this test needs a default-off feature"
        );

        let mut policy = FeaturePolicy::new();
        policy.set_local_override(key, true);
        assert!(
            policy.is_enabled(key),
            "asking for it overrides the default"
        );

        policy.set_capability_missing(key);
        assert_eq!(
            policy.decide(key).reason,
            Some(FallbackReason::CapabilityMissing),
            "a device without the capability still cannot run it"
        );
    }

    #[test]
    fn turning_a_feature_off_beats_every_other_layer() {
        let mut policy = FeaturePolicy::new();
        let key = FeatureKey::PresentSwapDamage;
        policy.set_local_override(key, false);
        policy.set_capability_missing(key);
        assert_eq!(
            policy.decide(key).reason,
            Some(FallbackReason::LocalOverride),
            "the kill switch is what stopped it, and that is what must be reported"
        );
    }

    #[test]
    fn a_default_off_feature_says_so_rather_than_blaming_the_device() {
        let policy = FeaturePolicy::new();
        assert_eq!(
            policy.decide(FeatureKey::CanvasDrawAtlas).reason,
            Some(FallbackReason::DefaultOff)
        );
    }

    #[test]
    fn override_lists_reject_typos_instead_of_ignoring_them() {
        let mut policy = FeaturePolicy::new();
        assert!(policy.apply_override_list("canvas_draw_atlas=on").is_ok());
        assert!(policy.is_enabled(FeatureKey::CanvasDrawAtlas));

        assert!(policy.apply_override_list("canvas_draw_atlass=on").is_err());
        assert!(policy.apply_override_list("canvas_draw_atlas=yes").is_err());
        assert!(policy.apply_override_list("canvas_draw_atlas").is_err());
    }
}
