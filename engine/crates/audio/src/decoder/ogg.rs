use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;
use shared::error::{EngineError, EngineResult, ErrorCode};

use super::DecodedAudio;

pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    let cursor = Cursor::new(data);
    let mut reader = OggStreamReader::new(cursor).map_err(|e| {
        EngineError::from_detail(
            ErrorCode::InvalidArgument,
            format!("OGG decode error: {:?}", e),
        )
    })?;

    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channels = reader.ident_hdr.audio_channels as u32;

    // Estimate capacity from file size to reduce reallocations.
    // OGG Vorbis typical compression ratio is ~10:1 for stereo 44.1kHz,
    // so decoded f32 samples ~ data.len() * 10 / 4 (bytes → f32).
    let estimated_samples = data.len() * 2; // conservative: ~8:1 ratio
    let mut samples: Vec<f32> = Vec::with_capacity(estimated_samples);

    while let Some(packet) = reader
        .read_dec_packet_generic::<Vec<Vec<f32>>>()
        .map_err(|e| {
            EngineError::from_detail(
                ErrorCode::InvalidArgument,
                format!("OGG decode error: {:?}", e),
            )
        })?
    {
        // packet is Vec<Vec<f32>> - one Vec<f32> per channel
        // Interleave the channels
        if packet.is_empty() {
            continue;
        }

        let frame_count = packet[0].len();
        // Reject a decode bomb before reserving/growing the buffer.
        if !crate::limits::pcm_samples_within_budget(samples.len() + frame_count * packet.len()) {
            return Err(EngineError::from_detail(
                ErrorCode::InvalidArgument,
                "OGG decode exceeds the PCM budget",
            ));
        }
        samples.reserve(frame_count * packet.len());
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
