use minimp3::{Decoder, Frame};
use shared::error::{EngineError, EngineResult, ErrorCode};

use super::DecodedAudio;

pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    let mut decoder = Decoder::new(data);

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate = 0u32;
    let mut channels = 0u32;

    loop {
        match decoder.next_frame() {
            Ok(Frame {
                data: frame_data,
                sample_rate: sr,
                channels: ch,
                ..
            }) => {
                if sample_rate == 0 {
                    sample_rate = sr as u32;
                    channels = ch as u32;
                }

                // Convert i16 to f32
                for s in frame_data {
                    samples.push(s as f32 / 32768.0);
                }
            }
            Err(minimp3::Error::Eof) => break,
            Err(e) => {
                return Err(EngineError::from_detail(
                    ErrorCode::InvalidArgument,
                    format!("MP3 decode error: {:?}", e),
                ));
            }
        }
    }

    if samples.is_empty() {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "MP3 decode produced no samples",
        ));
    }

    Ok(DecodedAudio {
        samples,
        sample_rate,
        channels,
    })
}
