use std::ffi::c_int;

use minimp3::ffi;
use shared::error::{EngineError, EngineResult, ErrorCode};

use super::DecodedAudio;

/// Largest MPEG-1 Layer III frame in bytes (320 kb/s at 32 kHz, padded).
const MAX_FRAME_BYTES: usize = 1441;

/// Bytes an MP3 frame header occupies.
const HEADER_BYTES: usize = 4;

/// Bytes that must stay unread behind the decode cursor while a stream's buffer
/// is still growing.
///
/// minimp3 keeps `mp3dec_t` -- the bit reservoir included -- only when it can
/// confirm the *next* frame's header in the same buffer; when it cannot, it
/// wipes the decoder and searches from scratch. Decoding right up to the end of
/// a partially received buffer therefore throws the reservoir away at every
/// chunk boundary, and an MP3 frame whose main data lives in that reservoir then
/// decodes to nothing at all. Two maximum frames of slack keeps the confirmation
/// available; at 128 kb/s that is under a quarter of a second of hold-back, well
/// inside the half second of buffering the player already waits for.
pub(crate) const STREAM_LOOKAHEAD_BYTES: usize = 2 * MAX_FRAME_BYTES;

/// What one call to [`Mp3FrameDecoder::decode`] did with the bytes it was given.
pub(crate) enum Mp3Step<'a> {
    /// A frame decoded. `pcm` is interleaved and borrowed from the decoder's own
    /// buffer, so it is valid until the next `decode`.
    Frame {
        pcm: &'a [i16],
        sample_rate: u32,
        channels: u32,
        consumed: usize,
    },
    /// Bytes carried no audio -- an ID3 tag, padding, or garbage -- and belong to
    /// nobody. Skipping them is progress.
    Skipped(usize),
    /// Not enough input to decide anything. The caller must keep what it has.
    NeedMoreData,
}

/// One minimp3 decoder that lives as long as the stream does, plus the fixed
/// buffer a frame decodes into.
///
/// **Both halves of that sentence are load-bearing.** Constructing a decoder per
/// chunk allocated three times over -- the 6.6 KiB `mp3dec_t`, a
/// virtual-memory-backed ring, and an 11 KiB refill buffer -- and, worse, reset
/// the bit reservoir and the MDCT overlap that MP3 frames are entitled to
/// inherit from their predecessors. Decoding into a buffer the decoder owns is
/// what removes the per-frame `Vec<i16>` the safe wrapper allocates and hands
/// back; at 1152 samples a frame that is one allocation every 26 ms of audio.
pub(crate) struct Mp3FrameDecoder {
    state: Box<ffi::mp3dec_t>,
    /// Sized by minimp3's documented maximum for a single frame, which is the
    /// contract the `pcm` pointer below is passed under.
    pcm: Box<[i16; minimp3::MAX_SAMPLES_PER_FRAME]>,
}

impl Mp3FrameDecoder {
    pub(crate) fn new() -> Self {
        // `mp3dec_init` only invalidates the cached header; the rest of the
        // struct is the decoder's state and must start zeroed.
        let mut state = Box::new(ffi::mp3dec_t {
            mdct_overlap: [[0.0; 288]; 2],
            qmf_state: [0.0; 960],
            reserv: 0,
            free_format_bytes: 0,
            header: [0; 4],
            reserv_buf: [0; 511],
        });
        // SAFETY: `state` is a live, correctly aligned, fully initialised
        // `mp3dec_t` owned by this struct.
        unsafe { ffi::mp3dec_init(&mut *state) };

        Self {
            state,
            pcm: Box::new([0; minimp3::MAX_SAMPLES_PER_FRAME]),
        }
    }

    /// Decode at most one frame from the front of `input`.
    ///
    /// The whole remaining buffer should be passed, not one frame's worth:
    /// minimp3's state-preserving fast path is conditional on the next header
    /// being visible.
    pub(crate) fn decode<'a>(&'a mut self, input: &[u8]) -> Mp3Step<'a> {
        if input.is_empty() {
            return Mp3Step::NeedMoreData;
        }

        let mut info = ffi::mp3dec_frame_info_t {
            frame_bytes: 0,
            frame_offset: 0,
            channels: 0,
            hz: 0,
            layer: 0,
            bitrate_kbps: 0,
        };

        // SAFETY: `input` is a readable slice of `len` bytes and `len` is its own
        // length clamped to what the C signature can carry; `pcm` points at
        // `MINIMP3_MAX_SAMPLES_PER_FRAME` writable samples, which is the maximum
        // one frame can produce and the size minimp3 documents for this pointer;
        // `state` is an initialised `mp3dec_t` owned here and borrowed uniquely
        // for this call. Nothing borrowed escapes the call.
        let len = input.len().min(c_int::MAX as usize) as c_int;
        let samples_per_channel = unsafe {
            ffi::mp3dec_decode_frame(
                &mut *self.state,
                input.as_ptr(),
                len,
                self.pcm.as_mut_ptr(),
                &mut info,
            )
        };

        let consumed = info.frame_bytes.max(0) as usize;

        if samples_per_channel <= 0 {
            // Two different answers arrive here and they must not be conflated.
            //
            // When minimp3 consumed *less* than it was given, it got past a real
            // frame it could not decode. Those bytes are behind us.
            //
            // When it claims everything it looked at, it means "nothing usable in
            // here" — and that is not the same as "these bytes are yours to
            // discard". A frame that simply has not arrived in full looks
            // identical from here. Any frame beginning within the last
            // `MAX_FRAME_BYTES` may still be incomplete, so that much is kept,
            // plus a header's worth minus one for a sync word straddling the
            // boundary. Everything older than that cannot be an incomplete
            // frame's start, because its frame would have fit.
            //
            // The bound is what keeps a stream of pure garbage from growing the
            // buffer without limit, and leaving nothing to skip is what stops
            // this from looping: the caller must wait for more input.
            let hold_back = if consumed >= input.len() {
                MAX_FRAME_BYTES + HEADER_BYTES - 1
            } else {
                0
            };
            return match consumed.saturating_sub(hold_back) {
                0 => Mp3Step::NeedMoreData,
                skip => Mp3Step::Skipped(skip),
            };
        }

        let channels = info.channels.max(1) as usize;
        let total = (samples_per_channel as usize).saturating_mul(channels);
        debug_assert!(total <= minimp3::MAX_SAMPLES_PER_FRAME);

        Mp3Step::Frame {
            pcm: &self.pcm[..total.min(minimp3::MAX_SAMPLES_PER_FRAME)],
            sample_rate: info.hz.max(0) as u32,
            channels: channels as u32,
            consumed,
        }
    }
}

impl Mp3FrameDecoder {
    /// Byte length of the frame at the front of `input`, without decoding it.
    ///
    /// minimp3 answers this directly when handed a null output pointer: it
    /// locates the frame, records its length, and returns before any of the work
    /// that depends on decoder state.
    ///
    /// **That last part is the whole reason this exists.** A frame whose main
    /// data lives in a bit reservoir the decoder does not have decodes to zero
    /// samples, which from outside is indistinguishable from garbage — so a probe
    /// that measured by decoding would report "no frame here" for exactly the
    /// frames worth rescuing.
    fn frame_bytes_at_front(&mut self, input: &[u8]) -> Option<usize> {
        if input.is_empty() {
            return None;
        }

        let mut info = ffi::mp3dec_frame_info_t {
            frame_bytes: 0,
            frame_offset: 0,
            channels: 0,
            hz: 0,
            layer: 0,
            bitrate_kbps: 0,
        };

        // SAFETY: as in `decode`, except for the output pointer. minimp3 checks
        // `pcm` for null and returns the frame's sample count before writing
        // anything through it, so passing null selects its measure-only path.
        let len = input.len().min(c_int::MAX as usize) as c_int;
        let samples = unsafe {
            ffi::mp3dec_decode_frame(
                &mut *self.state,
                input.as_ptr(),
                len,
                std::ptr::null_mut(),
                &mut info,
            )
        };

        (samples > 0 && info.frame_bytes > 0).then_some(info.frame_bytes as usize)
    }
}

/// Length of the frame at the front of `buffer`, if one is there in full.
///
/// **Why this exists.** minimp3 will not accept a frame it cannot chain to a
/// successor — except when the buffer it is handed is *exactly* one frame, which
/// it accepts outright. That exception is the only way past a stream whose next
/// bytes are a tag rather than a frame, and an ID3v1 tag is 128 bytes at the end
/// of a large fraction of real MP3 files.
///
/// Growing prefixes rather than a header table: asking minimp3 what a frame's
/// length is means duplicating its bitrate and sample-rate tables here, and a
/// second opinion about frame lengths is a second thing to be wrong. `hint` is
/// the previous frame's length, which a constant-bitrate stream repeats, so the
/// usual cost is one probe rather than a scan.
///
/// **The decoder passed in must be a throwaway.** Every rejected probe resets
/// minimp3's state, and the bit reservoir is precisely what the real decoder has
/// to keep.
pub(crate) fn first_frame_length(
    probe: &mut Mp3FrameDecoder,
    buffer: &[u8],
    hint: Option<usize>,
) -> Option<usize> {
    let mut ends_a_frame =
        |len: usize| len <= buffer.len() && probe.frame_bytes_at_front(&buffer[..len]) == Some(len);

    if let Some(len) = hint
        && ends_a_frame(len)
    {
        return Some(len);
    }

    // No frame can be longer than this, so failing to find one by here means
    // there is not a whole frame at the front.
    let limit = buffer.len().min(MAX_FRAME_BYTES);
    (HEADER_BYTES..=limit).find(|&len| ends_a_frame(len))
}

/// Convert one interleaved `i16` frame to normalized `f32`, appending.
#[inline]
pub(crate) fn append_as_f32(pcm: &[i16], out: &mut Vec<f32>) {
    out.extend(pcm.iter().map(|&sample| f32::from(sample) / 32768.0));
}

pub fn decode(data: &[u8]) -> EngineResult<DecodedAudio> {
    let mut frames = Mp3FrameDecoder::new();

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate = 0u32;
    let mut channels = 0u32;
    let mut pos = 0usize;

    while pos < data.len() {
        match frames.decode(&data[pos..]) {
            Mp3Step::NeedMoreData => break,
            Mp3Step::Skipped(skipped) => pos += skipped,
            Mp3Step::Frame {
                pcm,
                sample_rate: sr,
                channels: ch,
                consumed,
            } => {
                pos += consumed;
                if sample_rate == 0 {
                    sample_rate = sr;
                    channels = ch;
                }

                // Reject a decode bomb before growing the buffer.
                if !crate::limits::pcm_samples_within_budget(
                    samples.len().saturating_add(pcm.len()),
                ) {
                    return Err(EngineError::from_detail(
                        ErrorCode::InvalidArgument,
                        "MP3 decode exceeds the PCM budget",
                    ));
                }

                append_as_f32(pcm, &mut samples);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp3_fixture;

    /// The fixture has to be a real MP3 before anything built on it means
    /// something, so this is the control: a stream of frames decodes to exactly
    /// the sample count its headers promise.
    #[test]
    fn the_synthetic_stream_decodes_to_the_frames_its_headers_declare() {
        const FRAMES: usize = 4;
        let decoded = decode(&mp3_fixture::stream(FRAMES)).expect("fixture must decode");

        assert_eq!(decoded.sample_rate, mp3_fixture::SAMPLE_RATE);
        assert_eq!(decoded.channels, mp3_fixture::CHANNELS as u32);
        assert_eq!(
            decoded.samples.len(),
            FRAMES * mp3_fixture::SAMPLES_PER_FRAME,
            "every frame must have produced a full frame of samples"
        );
    }

    /// The decoder's state is what a frame's main data may live in, and a frame
    /// that points into a reservoir a reset decoder does not have decodes to
    /// nothing. Feeding the same bytes one frame at a time to one decoder must
    /// therefore give the same answer as feeding them all at once.
    #[test]
    fn a_persistent_decoder_carries_the_bit_reservoir_across_calls() {
        const FRAMES: usize = 4;
        let stream = mp3_fixture::stream(FRAMES);

        let mut persistent = Mp3FrameDecoder::new();
        let mut carried = 0usize;
        let mut pos = 0usize;
        while pos < stream.len() {
            match persistent.decode(&stream[pos..]) {
                Mp3Step::NeedMoreData => break,
                Mp3Step::Skipped(skipped) => pos += skipped,
                Mp3Step::Frame { pcm, consumed, .. } => {
                    pos += consumed;
                    carried += pcm.len();
                }
            }
        }
        assert_eq!(carried, FRAMES * mp3_fixture::SAMPLES_PER_FRAME);

        // The same bytes, but with the decoder rebuilt for each frame -- which is
        // what constructing a decoder per network chunk amounted to. Every frame
        // after the first is silently lost.
        let mut rebuilt = 0usize;
        for index in 0..FRAMES {
            let frame = &stream[index * mp3_fixture::FRAME_BYTES..][..mp3_fixture::FRAME_BYTES];
            if let Mp3Step::Frame { pcm, .. } = Mp3FrameDecoder::new().decode(frame) {
                rebuilt += pcm.len();
            }
        }
        assert_eq!(
            rebuilt,
            mp3_fixture::SAMPLES_PER_FRAME,
            "a decoder rebuilt per frame must lose every frame that needs the \
             reservoir -- if this ever stops being true the test above proves \
             nothing and the fixture needs a stronger frame"
        );
    }

    /// Leading bytes that are not a frame -- an ID3 tag is the usual one -- must
    /// be got past without being mistaken for audio.
    ///
    /// minimp3 folds them into the frame it eventually finds rather than
    /// reporting them separately, so `consumed` covers both and the caller must
    /// advance by it rather than by the frame length it might have assumed.
    #[test]
    fn leading_non_audio_bytes_are_consumed_with_the_frame_that_follows_them() {
        const PADDING: usize = 64;
        let mut input = vec![0u8; PADDING];
        input.extend_from_slice(&mp3_fixture::stream(3));

        let mut decoder = Mp3FrameDecoder::new();
        let Mp3Step::Frame { pcm, consumed, .. } = decoder.decode(&input) else {
            panic!("the frame after the padding must be found");
        };
        assert_eq!(pcm.len(), mp3_fixture::SAMPLES_PER_FRAME);
        assert_eq!(
            consumed,
            PADDING + mp3_fixture::FRAME_BYTES,
            "the padding must be consumed along with the frame it precedes"
        );
    }

    /// Bytes that might still turn into a frame must survive to be re-offered
    /// with the next chunk; consuming them would desynchronise the stream.
    ///
    /// The three cases are the three ways "no frame yet" arises, and the middle
    /// one is the one that was got wrong: a frame's first bytes are
    /// indistinguishable, from minimp3's answer alone, from bytes worth nothing.
    #[test]
    fn bytes_that_may_still_become_a_frame_are_held_back() {
        // A sync word split across a chunk boundary.
        assert!(matches!(
            Mp3FrameDecoder::new().decode(&[0xFF, 0xFB, 0x90]),
            Mp3Step::NeedMoreData
        ));

        // The front of a real frame that has not fully arrived. Discarding this
        // is silent data loss, and minimp3 reports it exactly as it reports
        // garbage: by claiming every byte it looked at.
        let frame = mp3_fixture::stream(1);
        let partial = &frame[..mp3_fixture::FRAME_BYTES / 3];
        assert!(matches!(
            Mp3FrameDecoder::new().decode(partial),
            Mp3Step::NeedMoreData
        ));

        // Pure noise must still make progress, or a stream opening with a large
        // tag would grow the buffer without limit. What is retained is bounded by
        // the largest frame that could still be incomplete.
        let noise = vec![0x5Au8; 4096];
        let retained = MAX_FRAME_BYTES + HEADER_BYTES - 1;
        assert!(matches!(
            Mp3FrameDecoder::new().decode(&noise),
            Mp3Step::Skipped(skipped) if skipped == noise.len() - retained
        ));
    }
}
