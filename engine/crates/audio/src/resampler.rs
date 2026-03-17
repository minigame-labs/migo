use rubato::{FftFixedIn, Resampler as RubatoResampler};
use shared::error::{EngineError, EngineResult, ErrorCode};

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
        let output_frames =
            ((input_frames as u64 * self.output_rate as u64) / self.input_rate as u64) as usize;

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

/// Resample audio to target sample rate if needed
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

    // Calculate output size
    let resample_ratio = target_sample_rate as f64 / audio.sample_rate as f64;
    let output_frames = (input_frames as f64 * resample_ratio).ceil() as usize;

    // Create resampler
    // chunk_size should be reasonably sized for efficiency
    let chunk_size = 1024.min(input_frames);

    let mut resampler = FftFixedIn::<f32>::new(
        audio.sample_rate as usize,
        target_sample_rate as usize,
        chunk_size,
        2, // sub_chunks for quality
        channels,
    )
    .map_err(|e| {
        EngineError::from_detail(
            ErrorCode::Internal,
            format!("Failed to create resampler: {}", e),
        )
    })?;

    // De-interleave input samples into separate channel buffers
    let mut input_channels: Vec<Vec<f32>> = vec![Vec::with_capacity(input_frames); channels];
    for (i, sample) in audio.samples.iter().enumerate() {
        input_channels[i % channels].push(*sample);
    }

    // Process audio in chunks
    let mut output_channels: Vec<Vec<f32>> = vec![Vec::with_capacity(output_frames); channels];
    let frames_needed = resampler.input_frames_next();

    let mut pos = 0;
    while pos < input_frames {
        let end = (pos + frames_needed).min(input_frames);
        let chunk_len = end - pos;

        // Prepare input chunk
        let input_chunk: Vec<&[f32]> = input_channels.iter().map(|ch| &ch[pos..end]).collect();

        // Handle partial last chunk by padding with zeros
        let padded_input: Vec<Vec<f32>>;
        let actual_input: Vec<&[f32]> = if chunk_len < frames_needed {
            padded_input = input_chunk
                .iter()
                .map(|ch| {
                    let mut padded = ch.to_vec();
                    padded.resize(frames_needed, 0.0);
                    padded
                })
                .collect();
            padded_input.iter().map(|v| v.as_slice()).collect()
        } else {
            input_chunk
        };

        // Resample
        let output_chunk = resampler.process(&actual_input, None).map_err(|e| {
            EngineError::from_detail(ErrorCode::Internal, format!("Resampling failed: {}", e))
        })?;

        // Append output
        for (ch_idx, ch_data) in output_chunk.iter().enumerate() {
            output_channels[ch_idx].extend_from_slice(ch_data);
        }

        pos = end;
    }

    // Interleave output channels back into a single buffer
    let actual_output_frames = output_channels[0].len();
    let mut output_samples = Vec::with_capacity(actual_output_frames * channels);
    for frame_idx in 0..actual_output_frames {
        for ch in &output_channels {
            output_samples.push(ch[frame_idx]);
        }
    }

    tracing::debug!(
        "Resampling complete: {} input frames -> {} output frames",
        input_frames,
        actual_output_frames
    );

    Ok(DecodedAudio {
        samples: output_samples,
        sample_rate: target_sample_rate,
        channels: audio.channels,
    })
}
