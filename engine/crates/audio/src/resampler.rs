use shared::error::{EngineError, EngineResult, ErrorCode};

use crate::decoder::DecodedAudio;

/// Streaming linear-interpolation resampler with an exact rational step.
///
/// The read position is kept as a whole frame count plus a fraction expressed as
/// `pos_num / output_rate`, advanced by adding `input_rate` and carrying. That is
/// exact for every rate pair. The step used to be a 16.16 fixed-point ratio,
/// which truncates: 44.1 kHz to 48 kHz came out 3.3e-6 short, so a three-minute
/// track finished about 28 frames early and a resampled loop was never
/// sample-exact.
pub struct StreamResampler {
    channels: usize,
    input_rate: u64,
    output_rate: u64,
    /// Whole-frame read position within the virtual input, which is
    /// `[previous chunk's last frame] ++ current chunk`.
    pos_frames: u64,
    /// Fractional read position, as `pos_num / output_rate`.
    pos_num: u64,
    /// Last input frame of the previous chunk (`channels` samples), the virtual
    /// prefix that keeps interpolation continuous across a chunk boundary. Empty
    /// before the first chunk.
    ///
    /// One buffer, refilled in place. It used to be rebuilt with `to_vec` on every
    /// call, which on the streaming path meant one allocation per MP3 frame for as
    /// few as two samples.
    history: Vec<f32>,
}

impl StreamResampler {
    pub fn new(input_rate: u32, output_rate: u32, channels: u32) -> Self {
        Self {
            channels: channels as usize,
            input_rate: input_rate.max(1) as u64,
            output_rate: output_rate.max(1) as u64,
            pos_frames: 0,
            pos_num: 0,
            history: Vec::with_capacity(channels as usize),
        }
    }

    /// Advance the read position by exactly one output frame.
    #[inline]
    fn advance(&mut self) {
        self.pos_num += self.input_rate;
        self.pos_frames += self.pos_num / self.output_rate;
        self.pos_num %= self.output_rate;
    }

    /// Resample `input`, **appending** the result to `output`.
    ///
    /// Streaming-safe: an output frame whose right-hand interpolation neighbour
    /// would fall in the *next* chunk is deferred (the read position carries over)
    /// rather than approximated, so concatenating the per-chunk outputs equals a
    /// single-pass resample of the whole stream -- no sample is skipped or
    /// duplicated at a chunk boundary. Call [`Self::flush_into`] once at the end of
    /// the stream to emit the frame that has no successor at all.
    ///
    /// Appending into a caller-owned buffer is the only shape offered on purpose.
    /// The convenience wrapper that returned a fresh `Vec` was the streaming
    /// decoder's per-frame allocation, and an allocating call left available is one
    /// a future caller will reach for.
    pub fn process_into(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let channels = self.channels;
        if channels == 0 || input.is_empty() {
            return;
        }

        let n_chunk = input.len() / channels;
        if n_chunk == 0 {
            // Not even one full frame; leave the carried-over prefix untouched.
            return;
        }

        let prefix = std::mem::take(&mut self.history);
        let has_prefix = !prefix.is_empty();
        let work_len = n_chunk + usize::from(has_prefix);

        let virtual_at = |frame: usize, ch: usize| -> f32 {
            match (has_prefix, frame) {
                (true, 0) => prefix[ch],
                (true, _) => input[(frame - 1) * channels + ch],
                (false, _) => input[frame * channels + ch],
            }
        };

        let est_frames = (n_chunk as u64 * self.output_rate) / self.input_rate;
        output.reserve((est_frames as usize + 1) * channels);

        while (self.pos_frames as usize) + 1 < work_len {
            let frame = self.pos_frames as usize;
            let frac = self.pos_num as f32 / self.output_rate as f32;
            for ch in 0..channels {
                let s0 = virtual_at(frame, ch);
                let s1 = virtual_at(frame + 1, ch);
                output.push(s0 + (s1 - s0) * frac);
            }
            self.advance();
        }

        // This chunk's last frame becomes virtual index 0 of the next chunk, so
        // rebase the read position by however many virtual frames were passed.
        let consumed = (work_len - 1) as u64;
        self.pos_frames = self.pos_frames.saturating_sub(consumed);

        let last_frame_start = (n_chunk - 1) * channels;
        let mut prefix = prefix;
        prefix.clear();
        prefix.extend_from_slice(&input[last_frame_start..last_frame_start + channels]);
        self.history = prefix;
    }

    /// Emit the output frames that were waiting on a successor that will never
    /// arrive, holding the final input frame.
    ///
    /// **Without this the end of every resampled buffer was silently dropped.**
    /// `process_into` defers any output frame whose right-hand neighbour is not in
    /// the chunk yet, which is correct mid-stream and wrong at the end: the last
    /// frames were deferred forever. A resampled loop therefore restarted a frame
    /// or two early, which is a step discontinuity at the seam -- an audible click
    /// on every repeat.
    pub fn flush_into(&mut self, output: &mut Vec<f32>) {
        let channels = self.channels;
        if channels == 0 || self.history.len() < channels {
            return;
        }
        // `history` is the whole remaining virtual input: one frame, at index 0.
        while self.pos_frames == 0 {
            for ch in 0..channels {
                // No successor, so the final frame is held rather than
                // interpolated toward silence, which would fade the tail out.
                // (`process_into` does `s0 + (s1 - s0) * frac`; here s1 == s0.)
                output.push(self.history[ch]);
            }
            self.advance();
        }
        self.history.clear();
    }
}

/// Resample audio to target sample rate if needed.
///
/// Linear interpolation -- sufficient for game audio (sound effects, BGM) and it
/// avoids pulling in the heavyweight `rubato` crate (~150-200KB SO).
pub fn resample_if_needed(
    audio: DecodedAudio,
    target_sample_rate: u32,
) -> EngineResult<DecodedAudio> {
    // No resampling needed if rates match
    if audio.sample_rate == target_sample_rate {
        return Ok(audio);
    }

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
    resampler.flush_into(&mut output_samples);

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
        // Every frame is asserted now: the tail is emitted rather than deferred
        // forever, and the last frame holds instead of interpolating toward silence.
        for k in 0..out.frame_count().saturating_sub(1) {
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
        let mut one_pass = StreamResampler::new(input_rate, output_rate, channels);
        one_pass.process_into(&samples, &mut single);
        one_pass.flush_into(&mut single);

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

    /// The step used to be a truncated 16.16 ratio, so 44.1 kHz to 48 kHz came out
    /// 3.3e-6 short: a three-minute track ended about 28 frames early, and a
    /// resampled loop was never sample-exact. An exact rational step cannot drift
    /// no matter how long the stream is.
    #[test]
    fn the_rate_ratio_does_not_drift_over_a_long_stream() {
        let input_rate = 44_100u32;
        let output_rate = 48_000u32;
        let input_frames = 44_100 * 180; // three minutes

        let mut resampler = StreamResampler::new(input_rate, output_rate, 1);
        let mut produced = 0usize;
        let chunk = vec![0.0f32; 4_410];
        let mut sink = Vec::new();
        let mut fed = 0usize;
        while fed < input_frames {
            sink.clear();
            resampler.process_into(&chunk, &mut sink);
            produced += sink.len();
            fed += chunk.len();
        }
        sink.clear();
        resampler.flush_into(&mut sink);
        produced += sink.len();

        let expected = input_frames as u64 * output_rate as u64 / input_rate as u64;
        let drift = (produced as i64 - expected as i64).abs();
        assert!(
            drift <= 1,
            "produced {produced} frames, expected {expected} ({drift} frames of drift)"
        );
    }

    /// A deferred tail is a shortened buffer, and a shortened loop restarts early
    /// -- a step discontinuity at the seam, audible as a click on every repeat.
    #[test]
    fn the_final_frames_are_emitted_rather_than_deferred_forever() {
        let audio = DecodedAudio {
            samples: vec![1.0f32; 100],
            sample_rate: 24_000,
            channels: 1,
        };
        let out = resample_if_needed(audio, 48_000).unwrap();

        assert_eq!(
            out.frame_count(),
            200,
            "2x upsampling 100 frames must yield 200, not 198"
        );
        assert_eq!(
            out.samples.last().copied(),
            Some(1.0),
            "the tail must hold the final sample, not fade toward silence"
        );
    }
}
