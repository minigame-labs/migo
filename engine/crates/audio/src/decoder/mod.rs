pub(crate) mod mp3;
mod ogg;
mod wav;

use shared::error::{EngineError, EngineResult, ErrorCode};

/// Decoded audio data in f32 samples, interleaved for multi-channel
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u32,
}

impl DecodedAudio {
    /// Number of sample frames (samples / channels)
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// Duration in seconds
    pub fn duration(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frame_count() as f64 / self.sample_rate as f64
        }
    }
}

/// Detect audio format and decode
pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    // Try to detect format by magic bytes
    if data.len() < 4 {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "Audio data too short to detect format",
        ));
    }

    // WAV: "RIFF"
    if data.starts_with(b"RIFF") {
        return wav::decode(data);
    }

    // OGG: "OggS"
    if data.starts_with(b"OggS") {
        return ogg::decode(data);
    }

    // MP3: ID3 tag or frame sync
    if data.starts_with(b"ID3") || (data[0] == 0xFF && (data[1] & 0xE0) == 0xE0) {
        return mp3::decode(data);
    }

    Err(EngineError::from_detail(
        ErrorCode::InvalidArgument,
        "Unknown audio format",
    ))
}
