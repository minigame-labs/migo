use shared::error::EngineResult;

use crate::decoder::DecodedAudio;

/// Simple streaming resampler using linear interpolation
/// Suitable for real-time streaming where low latency is more important than quality
pub struct StreamResampler {
    input_rate: u32,
    output_rate: u32,
    channels: u32,
    /// Fractional position in input (16.16 fixed point per channel)
    position: u64,
    /// Rate ratio as 16.16 fixed point
    ratio: u64,
    /// Last sample per channel for interpolation
    last_samples: Vec<f32>,
}

impl StreamResampler {
    pub fn new(input_rate: u32, output_rate: u32, channels: u32) -> Self {
        // Calculate ratio as 16.16 fixed point
        let ratio = ((input_rate as u64) << 16) / output_rate as u64;

        Self {
            input_rate,
            output_rate,
            channels,
            position: 0,
            ratio,
            last_samples: vec![0.0; channels as usize],
        }
    }

    /// Process interleaved samples, returns resampled interleaved output
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let channels = self.channels as usize;
        if channels == 0 || input.is_empty() {
            return Vec::new();
        }

        let input_frames = input.len() / channels;
        // Ceiling division: generate enough output frames to consume ALL input.
        // Floor division would leave a fractional frame unconsumed per chunk,
        // causing cumulative drift (~28ms over 3 minutes for 44.1→48kHz).
        let output_frames =
            ((input_frames as u64 * self.output_rate as u64 + self.input_rate as u64 - 1)
                / self.input_rate as u64) as usize;

        if output_frames == 0 {
            return Vec::new();
        }

        let mut output = Vec::with_capacity(output_frames * channels);

        for _ in 0..output_frames {
            let pos_int = (self.position >> 16) as usize;
            let frac = (self.position & 0xFFFF) as f32 / 65536.0;

            for ch in 0..channels {
                let idx0 = pos_int * channels + ch;
                let idx1 = (pos_int + 1) * channels + ch;

                let s0 = if pos_int == 0 && self.position < (1 << 16) {
                    self.last_samples[ch]
                } else if idx0 < input.len() {
                    input[idx0]
                } else {
                    0.0
                };

                let s1 = if idx1 < input.len() {
                    input[idx1]
                } else if idx0 < input.len() {
                    input[idx0]
                } else {
                    0.0
                };

                // Linear interpolation
                output.push(s0 + (s1 - s0) * frac);
            }

            self.position += self.ratio;
        }

        // Save last samples for next chunk
        if input_frames > 0 {
            let last_frame_start = (input_frames - 1) * channels;
            for ch in 0..channels {
                self.last_samples[ch] = input[last_frame_start + ch];
            }
        }

        // Keep only fractional part of position for next chunk
        let frames_consumed = self.position >> 16;
        if frames_consumed >= input_frames as u64 {
            self.position = self.position - ((input_frames as u64) << 16);
        }

        output
    }
}

/// Resample audio to target sample rate if needed.
///
/// Uses linear interpolation — sufficient for game audio (sound effects, BGM).
/// This avoids pulling in the heavyweight `rubato` crate (~150-200KB SO).
pub fn resample_if_needed(
    audio: DecodedAudio,
    target_sample_rate: u32,
) -> EngineResult<DecodedAudio> {
    // No resampling needed if rates match
    if audio.sample_rate == target_sample_rate {
        return Ok(audio);
    }

    tracing::debug!(
        "Resampling audio from {} Hz to {} Hz ({} channels, {} frames)",
        audio.sample_rate,
        target_sample_rate,
        audio.channels,
        audio.frame_count()
    );

    let channels = audio.channels as usize;
    let input_frames = audio.frame_count();

    if channels == 0 || input_frames == 0 {
        return Ok(audio);
    }

    let resample_ratio = target_sample_rate as f64 / audio.sample_rate as f64;
    let output_frames = (input_frames as f64 * resample_ratio).ceil() as usize;

    let mut resampler = StreamResampler::new(audio.sample_rate, target_sample_rate, audio.channels);

    // Process in chunks to bound memory for large files
    const CHUNK_FRAMES: usize = 8192;
    let chunk_samples = CHUNK_FRAMES * channels;
    let mut output_samples = Vec::with_capacity(output_frames * channels);

    let mut pos = 0;
    while pos < audio.samples.len() {
        let end = (pos + chunk_samples).min(audio.samples.len());
        let resampled = resampler.process(&audio.samples[pos..end]);
        output_samples.extend_from_slice(&resampled);
        pos = end;
    }

    tracing::debug!(
        "Resampling complete: {} input frames -> {} output frames",
        input_frames,
        output_samples.len() / channels
    );

    Ok(DecodedAudio {
        samples: output_samples,
        sample_rate: target_sample_rate,
        channels: audio.channels,
    })
}
