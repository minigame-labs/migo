use std::io::Cursor;

use hound::WavReader;
use shared::error::{EngineError, EngineResult, ErrorCode};

use super::DecodedAudio;

pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    let cursor = Cursor::new(data);
    let reader = WavReader::new(cursor).map_err(|e| {
        EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!("WAV decode error: {}", e),
        )
    })?;

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as u32;

    // Reject decode bombs up front: the header declares the total sample count,
    // so we can bail before allocating gigabytes of PCM.
    let declared_samples = reader.len() as usize;
    if !crate::limits::pcm_samples_within_budget(declared_samples) {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!(
                "WAV declares {} samples, exceeding the PCM budget",
                declared_samples
            ),
        ));
    }

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1i32 << (bits - 1)) as f32;

            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    Ok(DecodedAudio {
        samples,
        sample_rate,
        channels,
    })
}
