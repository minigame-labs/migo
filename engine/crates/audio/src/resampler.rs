use shared::error::{EngineError, EngineResult, ErrorCode};

use crate::decoder::DecodedAudio;

/// Simple streaming resampler using linear interpolation
/// Suitable for real-time streaming where low latency is more important than quality
pub struct StreamResampler {
    channels: u32,
    /// Fractional read position (16.16 fixed point) into the virtual input, which is
    /// `[previous chunk's last frame] ++ current chunk`.
    position: u64,
    /// Rate ratio as 16.16 fixed point (input_rate / output_rate).
    ratio: u64,
    /// Last input frame of the previous chunk (`channels` samples), used as the virtual
    /// prefix so interpolation stays continuous across chunk boundaries. Empty before
    /// the first chunk.
    ///
    /// One buffer, refilled in place. It used to be rebuilt with `to_vec` on every
    /// call, which on the streaming path meant one allocation per MP3 frame for
    /// as few as two samples.
    history: Vec<f32>,
}

impl StreamResampler {
    pub fn new(input_rate: u32, output_rate: u32, channels: u32) -> Self {
        // Rate ratio as 16.16 fixed point.
        let ratio = ((input_rate as u64) << 16) / output_rate as u64;

        Self {
            channels,
            position: 0,
            ratio,
            history: Vec::with_capacity(channels as usize),
        }
    }

    /// Resample `input`, **appending** the result to `output`.
    ///
    /// Streaming-safe: an output frame whose right-hand interpolation neighbour would
    /// fall in the *next* chunk is deferred (the read position carries over) rather than
    /// approximated, so concatenating the per-chunk outputs equals a single-pass resample
    /// of the whole stream — no sample is skipped or duplicated at a chunk boundary.
    ///
    /// Appending into a caller-owned buffer is the only shape offered on purpose.
    /// The convenience wrapper that returned a fresh `Vec` was the streaming
    /// decoder's per-frame allocation, and an allocating call left available is
    /// one a future caller will reach for.
    pub fn process_into(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let channels = self.channels as usize;
        if channels == 0 || input.is_empty() {
            return;
        }

        let n_chunk = input.len() / channels;
        if n_chunk == 0 {
            // Not even one full frame; leave the carried-over prefix untouched.
            return;
        }

        // Virtual input = [previous chunk's last frame] ++ this chunk. Taken out
        // rather than borrowed so the same allocation can be refilled below,
        // after the closure that reads it has gone.
        let mut prefix = std::mem::take(&mut self.history);
        let has_prefix = !prefix.is_empty();
        let work_len = n_chunk + usize::from(has_prefix);

        let virtual_at = |frame: usize, ch: usize| -> f32 {
            match (has_prefix, frame) {
                (true, 0) => prefix[ch],
                (true, _) => input[(frame - 1) * channels + ch],
                (false, _) => input[frame * channels + ch],
            }
        };

        let est_frames = ((n_chunk as u64) << 16) / self.ratio.max(1);
        output.reserve((est_frames as usize + 1) * channels);

        loop {
            let pos_int = (self.position >> 16) as usize;
            if pos_int + 1 >= work_len {
                break; // the successor sample belongs to the next chunk — defer it
            }
            let frac = (self.position & 0xFFFF) as f32 / 65536.0;
            for ch in 0..channels {
                let s0 = virtual_at(pos_int, ch);
                let s1 = virtual_at(pos_int + 1, ch);
                output.push(s0 + (s1 - s0) * frac);
            }
            self.position += self.ratio;
        }

        // This chunk's last frame becomes virtual index 0 of the next chunk, so rebase the
        // read position by however many virtual frames we advanced past.
        let consumed = (work_len - 1) as u64;
        self.position -= consumed << 16;

        let last_frame_start = (n_chunk - 1) * channels;
        prefix.clear();
        prefix.extend_from_slice(&input[last_frame_start..last_frame_start + channels]);
        self.history = prefix;
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

    // A within-budget input can still upsample past the PCM budget; reject before
    // allocating the (potentially huge) output buffer.
    let output_total = output_frames.saturating_mul(channels);
    if !crate::limits::pcm_samples_within_budget(output_total) {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!("resampled output ({output_total} samples) exceeds the PCM budget"),
        ));
    }

    let mut resampler = StreamResampler::new(audio.sample_rate, target_sample_rate, audio.channels);

    // Process in chunks to bound memory for large files, appending each chunk's
    // output directly into the destination (no per-chunk temporary + copy).
    const CHUNK_FRAMES: usize = 8192;
    let chunk_samples = CHUNK_FRAMES * channels;
    let mut output_samples = Vec::with_capacity(output_total);

    let mut pos = 0;
    while pos < audio.samples.len() {
        let end = (pos + chunk_samples).min(audio.samples.len());
        resampler.process_into(&audio.samples[pos..end], &mut output_samples);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::DecodedAudio;

    /// A linear ramp resampled with linear interpolation must stay a linear ramp:
    /// output[k] == k * input_rate / output_rate. Any deviation means input samples
    /// were skipped or duplicated — historically once per 8192-frame chunk boundary.
    #[test]
    fn upsample_ramp_has_no_chunk_boundary_glitch() {
        let input_rate = 24_000;
        let output_rate = 48_000; // integer 2x upsampling
        let n = 20_000; // > 2 * CHUNK_FRAMES so several chunk boundaries are crossed
        let samples: Vec<f32> = (0..n).map(|i| i as f32).collect();
        let audio = DecodedAudio {
            samples,
            sample_rate: input_rate,
            channels: 1,
        };

        let out = resample_if_needed(audio, output_rate).unwrap();

        let expected_len = n as usize * output_rate as usize / input_rate as usize;
        assert!(
            (out.frame_count() as i64 - expected_len as i64).abs() <= 3,
            "unexpected output length: got {}, want ~{}",
            out.frame_count(),
            expected_len
        );

        let step = input_rate as f64 / output_rate as f64; // 0.5 input frame per output frame
        let mut max_err = 0.0f64;
        let mut worst_k = 0usize;
        // The final 2 frames have no successor sample (end of stream) — skip them.
        for k in 0..out.frame_count().saturating_sub(2) {
            let want = k as f64 * step;
            let err = (out.samples[k] as f64 - want).abs();
            if err > max_err {
                max_err = err;
                worst_k = k;
            }
        }
        assert!(
            max_err < 0.01,
            "resample glitch: max error {:.3} at output frame {} (chunk boundary near k≈{})",
            max_err,
            worst_k,
            2 * 8192
        );
    }

    /// Chunking is only a memory optimisation: feeding the stream in 8192-frame chunks
    /// (as `resample_if_needed` does) must produce identical output to resampling the whole
    /// buffer in one call. This isolates chunk-boundary correctness from the inherent
    /// fixed-point ratio drift, and covers a non-integer ratio plus a stereo layout.
    #[test]
    fn chunked_resample_matches_single_pass() {
        let input_rate = 44_100;
        let output_rate = 48_000; // non-integer ratio
        let channels = 2u32;
        let n = 20_000usize;

        // A distinct ramp per channel so any channel bleed shows up.
        let mut samples = Vec::with_capacity(n * channels as usize);
        for i in 0..n {
            samples.push(i as f32); // channel 0
            samples.push(-(i as f32) * 2.0); // channel 1
        }

        let mut single = Vec::new();
        StreamResampler::new(input_rate, output_rate, channels).process_into(&samples, &mut single);

        let audio = DecodedAudio {
            samples,
            sample_rate: input_rate,
            channels,
        };
        let chunked = resample_if_needed(audio, output_rate).unwrap();

        assert_eq!(
            chunked.samples.len(),
            single.len(),
            "chunked length {} != single-pass length {}",
            chunked.samples.len(),
            single.len()
        );
        for (i, (chunk_s, single_s)) in chunked.samples.iter().zip(single.iter()).enumerate() {
            assert!(
                (chunk_s - single_s).abs() < 1e-6,
                "sample {} differs: chunked={} single-pass={}",
                i,
                chunk_s,
                single_s
            );
        }
    }

    /// A within-budget input that upsamples past the PCM budget must be rejected
    /// *before* the giant output buffer is allocated (the guard runs first).
    #[test]
    fn resample_rejects_over_budget_upsampling() {
        // 600k mono frames (~2.4 MB) upsampled 256x (3kHz -> 768kHz) => ~153M
        // output samples, above the 64 MiB / 4 = 16M-sample budget.
        let input_frames = 600_000usize;
        let audio = DecodedAudio {
            samples: vec![0.0f32; input_frames],
            sample_rate: 3_000,
            channels: 1,
        };
        assert!(
            resample_if_needed(audio, 768_000).is_err(),
            "over-budget upsample must be rejected"
        );
    }
}
