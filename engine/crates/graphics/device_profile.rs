use crate::device_caps::{DeviceCapabilities, DeviceTier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRenderProfile {
    pub max_upload_jobs_per_frame: usize,
    pub max_upload_bytes_per_frame: usize,
    pub enable_partial_damage: bool,
    pub enable_layer_cache: bool,
}

impl DeviceRenderProfile {
    pub fn from_detected_device(caps: &DeviceCapabilities, api_level: u32) -> Self {
        let tier = caps.tier();
        Self::from_tier(caps, api_level, tier)
    }

    pub fn from_caps(caps: &DeviceCapabilities, api_level: u32, tier: DeviceTier) -> Self {
        let detected_tier = caps.tier();
        debug_assert_eq!(
            tier, detected_tier,
            "DeviceRenderProfile::from_caps tier must match detected device tier"
        );

        Self::from_tier(caps, api_level, detected_tier)
    }

    fn from_tier(caps: &DeviceCapabilities, api_level: u32, tier: DeviceTier) -> Self {
        if api_level <= 23 || tier == DeviceTier::TierB {
            return Self {
                max_upload_jobs_per_frame: 1,
                max_upload_bytes_per_frame: 512 * 1024,
                enable_partial_damage: false,
                enable_layer_cache: false,
            };
        }

        Self {
            max_upload_jobs_per_frame: 4,
            max_upload_bytes_per_frame: 4 * 1024 * 1024,
            enable_partial_damage: caps.has_fence_sync,
            enable_layer_cache: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api23_tier_b_device_uses_conservative_profile() {
        let caps = DeviceCapabilities {
            gles_version: (2, 0),
            has_pbo: false,
            has_fence_sync: false,
            has_compute: false,
            ahb_available: false,
            has_buffer_age: false,
            has_partial_update: false,
            compressed_format_support: crate::compressed_upload::CompressedFormatSupport {
                etc2: false,
                astc: false,
            },
        };

        let profile = DeviceRenderProfile::from_caps(&caps, 23, DeviceTier::TierB);
        assert_eq!(profile.max_upload_jobs_per_frame, 1);
        assert!(!profile.enable_partial_damage);
    }

    #[test]
    fn api24_tier_a_device_uses_aggressive_profile() {
        let caps = DeviceCapabilities {
            gles_version: (3, 0),
            has_pbo: true,
            has_fence_sync: true,
            has_compute: false,
            ahb_available: false,
            has_buffer_age: false,
            has_partial_update: false,
            compressed_format_support: crate::compressed_upload::CompressedFormatSupport {
                etc2: true,
                astc: false,
            },
        };

        let profile = DeviceRenderProfile::from_caps(&caps, 24, DeviceTier::TierA);

        assert_eq!(profile.max_upload_jobs_per_frame, 4);
        assert_eq!(profile.max_upload_bytes_per_frame, 4 * 1024 * 1024);
        assert!(profile.enable_partial_damage);
        assert!(profile.enable_layer_cache);
    }

    #[test]
    fn from_caps_uses_detected_tier_when_caller_input_disagrees() {
        let caps = DeviceCapabilities {
            gles_version: (2, 0),
            has_pbo: false,
            has_fence_sync: false,
            has_compute: false,
            ahb_available: false,
            has_buffer_age: false,
            has_partial_update: false,
            compressed_format_support: crate::compressed_upload::CompressedFormatSupport {
                etc2: false,
                astc: false,
            },
        };

        assert_eq!(caps.tier(), DeviceTier::TierB);

        let render_profile = caps.render_profile(24);
        let detected_profile = DeviceRenderProfile::from_detected_device(&caps, 24);

        assert_eq!(render_profile, detected_profile);
        assert_eq!(render_profile.max_upload_jobs_per_frame, 1);
        assert!(!render_profile.enable_partial_damage);

        let mismatched = std::panic::catch_unwind(|| {
            DeviceRenderProfile::from_caps(&caps, 24, DeviceTier::TierA)
        });

        if cfg!(debug_assertions) {
            assert!(mismatched.is_err());
        } else {
            let profile = mismatched.unwrap();
            assert_eq!(profile, render_profile);
        }
    }
}
