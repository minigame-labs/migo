//! Central limits for audio PCM allocation.
//!
//! Both the WebAudio `createBuffer` path and the `decodeAudioData` decode path
//! must agree on how large a single decoded/allocated PCM buffer may get. This
//! module is the single source of truth so the native side, the decoders, and
//! the JS shims (which mirror these constants) stay consistent — preventing
//! both integer-overflow-sized `createBuffer` requests and decode bombs (a tiny
//! compressed file expanding into gigabytes of PCM).

use shared::error::{EngineError, EngineResult, ErrorCode};

/// Hard ceiling on a single decoded/allocated PCM buffer (interleaved f32).
///
/// 512 MiB ≈ 46 min of 48 kHz stereo f32 — generous for game BGM/SFX while
/// rejecting decode bombs and overflow-sized `createBuffer` requests.
pub const MAX_AUDIO_PCM_BYTES: u64 = 512 * 1024 * 1024;

/// The same ceiling expressed in interleaved f32 samples (all channels).
pub const MAX_AUDIO_PCM_SAMPLES: u64 = MAX_AUDIO_PCM_BYTES / 4;

/// Maximum channel count accepted for a buffer (Web Audio permits up to 32).
pub const MAX_AUDIO_CHANNELS: u32 = 32;

/// Valid `createBuffer` sample-rate range in Hz (Web Audio: 3000..=768000).
pub const MIN_SAMPLE_RATE: u32 = 3_000;
pub const MAX_SAMPLE_RATE: u32 = 768_000;

/// Validate `createBuffer`-style parameters and the resulting PCM footprint.
///
/// Uses checked arithmetic so an overflowing `length * channels * 4` is rejected
/// with a structured error instead of panicking (debug) or wrapping to a
/// bogus-but-smaller allocation (release).
pub fn validate_buffer_alloc(channels: u32, length: u32, sample_rate: u32) -> EngineResult<()> {
    if channels == 0 || channels > MAX_AUDIO_CHANNELS {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!(
                "invalid channel count {} (must be 1..={})",
                channels, MAX_AUDIO_CHANNELS
            ),
        ));
    }
    if sample_rate < MIN_SAMPLE_RATE || sample_rate > MAX_SAMPLE_RATE {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!(
                "invalid sample rate {} (must be {}..={})",
                sample_rate, MIN_SAMPLE_RATE, MAX_SAMPLE_RATE
            ),
        ));
    }
    if length == 0 {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "buffer length must be >= 1",
        ));
    }

    let bytes = (length as u64)
        .checked_mul(channels as u64)
        .and_then(|samples| samples.checked_mul(4))
        .ok_or_else(|| {
            EngineError::from_detail(ErrorCode::InvalidArgument, "buffer size overflow")
        })?;

    if bytes > MAX_AUDIO_PCM_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!(
                "buffer of {} bytes exceeds the {} byte PCM budget",
                bytes, MAX_AUDIO_PCM_BYTES
            ),
        ));
    }

    Ok(())
}

/// Whether an interleaved sample count (in-progress or final) is within budget.
///
/// Used by the decoders to abort a decode bomb before it grows unbounded.
#[inline]
pub fn pcm_samples_within_budget(sample_count: usize) -> bool {
    sample_count as u64 <= MAX_AUDIO_PCM_SAMPLES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_buffer() {
        // 1 second of 48 kHz stereo — trivially within budget.
        assert!(validate_buffer_alloc(2, 48_000, 48_000).is_ok());
    }

    #[test]
    fn rejects_zero_channels() {
        assert!(validate_buffer_alloc(0, 48_000, 48_000).is_err());
    }

    #[test]
    fn rejects_too_many_channels() {
        assert!(validate_buffer_alloc(MAX_AUDIO_CHANNELS + 1, 48_000, 48_000).is_err());
    }

    #[test]
    fn rejects_zero_length() {
        assert!(validate_buffer_alloc(2, 0, 48_000).is_err());
    }

    #[test]
    fn rejects_bad_sample_rate() {
        assert!(validate_buffer_alloc(2, 100, 0).is_err());
        assert!(validate_buffer_alloc(2, 100, MAX_SAMPLE_RATE + 1).is_err());
    }

    #[test]
    fn rejects_length_channels_overflow() {
        // 8 channels * 600M frames * 4 bytes = 19.2 GB. The naive `(length *
        // channels) as usize` in u32 space overflows; validation must reject.
        assert!(validate_buffer_alloc(8, 600_000_000, 48_000).is_err());
        // Extreme: u32::MAX frames.
        assert!(validate_buffer_alloc(2, u32::MAX, 48_000).is_err());
    }

    #[test]
    fn rejects_over_budget_but_non_overflowing() {
        // 2ch * 200M frames * 4 = 1.6 GB: fits in u64 without overflow but
        // still exceeds the 512 MiB budget.
        assert!(validate_buffer_alloc(2, 200_000_000, 48_000).is_err());
    }

    #[test]
    fn pcm_samples_budget_boundary() {
        assert!(pcm_samples_within_budget(MAX_AUDIO_PCM_SAMPLES as usize));
        assert!(!pcm_samples_within_budget(
            MAX_AUDIO_PCM_SAMPLES as usize + 1
        ));
    }
}
