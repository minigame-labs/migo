use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;
use shared::error::{EngineError, EngineResult, ErrorCode};

use super::DecodedAudio;

pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    let cursor = Cursor::new(data);
    let mut reader = OggStreamReader::new(cursor).map_err(|e| {
        EngineError::from_detail(ErrorCode::InvalidArgument, format!("OGG decode error: {:?}", e))
    })?;

    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channels = reader.ident_hdr.audio_channels as u32;

    let mut samples: Vec<f32> = Vec::new();

    while let Some(packet) = reader.read_dec_packet_generic::<Vec<Vec<f32>>>().map_err(|e| {
        EngineError::from_detail(ErrorCode::InvalidArgument, format!("OGG decode error: {:?}", e))
    })? {
        // packet is Vec<Vec<f32>> - one Vec<f32> per channel
        // Interleave the channels
        if packet.is_empty() {
            continue;
        }

        let frame_count = packet[0].len();
        for i in 0..frame_count {
            for ch in &packet {
                if i < ch.len() {
                    samples.push(ch[i]);
                }
            }
        }
    }

    if samples.is_empty() {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "OGG decode produced no samples",
        ));
    }

    Ok(DecodedAudio {
        samples,
        sample_rate,
        channels,
    })
}
