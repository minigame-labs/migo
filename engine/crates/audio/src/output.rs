use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use shared::error::{EngineError, EngineResult, ErrorCode};
use tracing::{error, info};

/// Ring buffer size in sample frames (per channel)
/// 8192 frames at 48kHz = ~170ms buffer
const RING_BUFFER_FRAMES: usize = 8192;

/// Low watermark - when buffer falls below this, signal to refill
/// 2048 frames at 48kHz = ~42ms
const LOW_WATERMARK_FRAMES: usize = 2048;

/// High watermark - the refill *target*. A refill pass fills only up to here,
/// not to the top of the ring, so a freshly triggered sound isn't queued behind
/// ~150ms of already-buffered audio. 4096 frames at 48kHz = ~85ms, which still
/// leaves 2x the low-watermark margin against underruns.
const HIGH_WATERMARK_FRAMES: usize = 4096;

/// Lightweight signaling for callback-driven audio.
/// Uses atomic flag instead of Condvar for lower overhead.
///
/// # Atomic ordering model
///
/// The two atomics serve different synchronization purposes and intentionally
/// use different memory orderings:
///
/// - **`needs_data`** (`Release` on store, `AcqRel` on swap): This flag
///   establishes a happens-before relationship between the audio callback
///   (producer of the signal) and the audio thread (consumer).  When the
///   callback stores `true` with `Release`, all preceding ring buffer reads
///   (pop_slice, occupied_len) are visible to the audio thread after it
///   observes `true` via the `AcqRel` swap in `check_and_clear`.  This
///   ensures the audio thread sees the correct buffer state before refilling.
///
/// - **`buffer_level`** (`Relaxed`): This is a best-effort hint used for
///   monitoring and the `needs_data()` heuristic on `AudioOutput`.  It does
///   not guard any shared data -- the ring buffer itself is lock-free and
///   the producer/consumer halves are on separate threads.  A slightly stale
///   value is harmless (worst case: one extra or one missed refill cycle).
///   Using `Relaxed` avoids unnecessary memory fences on the hot audio
///   callback path.
///
/// - **`stream_error`** (`Release` on store, `Acquire` on load): The error
///   callback stores `true` with `Release` to ensure the error state is
///   visible to the audio thread checking `is_alive()` with `Acquire`.
#[derive(Clone)]
pub struct AudioSync {
    /// Flag indicating buffer needs data.
    /// Ordering: Release (store in callback) / AcqRel (swap in audio thread).
    needs_data: Arc<AtomicBool>,
    /// Current buffer fill level (in samples).
    /// Ordering: Relaxed -- advisory hint only, not used for synchronization.
    buffer_level: Arc<AtomicUsize>,
    /// Largest hardware callback request seen so far (in samples).
    /// Ordering: Relaxed -- advisory; lets the refill target cover a full callback.
    max_callback: Arc<AtomicUsize>,
}

impl AudioSync {
    fn new() -> Self {
        Self {
            needs_data: Arc::new(AtomicBool::new(false)),
            buffer_level: Arc::new(AtomicUsize::new(0)),
            max_callback: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Signal that buffer needs data (called from callback)
    #[inline]
    fn signal_need_data(&self) {
        self.needs_data.store(true, Ordering::Release);
    }

    /// Check and clear the needs_data flag
    #[inline]
    pub fn check_and_clear(&self) -> bool {
        self.needs_data.swap(false, Ordering::AcqRel)
    }

    #[inline]
    fn update_level(&self, level: usize) {
        self.buffer_level.store(level, Ordering::Relaxed);
    }

    /// Get current buffer fill level
    #[inline]
    pub fn buffer_level(&self) -> usize {
        self.buffer_level.load(Ordering::Relaxed)
    }

    /// Record the sample count of a hardware callback request (running max).
    #[inline]
    fn observe_callback(&self, len: usize) {
        self.max_callback.fetch_max(len, Ordering::Relaxed);
    }

    /// Largest hardware callback request seen so far (samples); 0 until the first callback.
    #[inline]
    pub fn max_callback(&self) -> usize {
        self.max_callback.load(Ordering::Relaxed)
    }
}

/// Audio output handle
pub struct AudioOutput {
    stream: Stream,
    producer: HeapProd<f32>,
    sample_rate: u32,
    channels: u32,
    sync: AudioSync,
    low_watermark_samples: usize,
    high_watermark_samples: usize,
    stream_error: Arc<AtomicBool>,
}

impl AudioOutput {
    /// Create a new audio output
    pub fn new() -> EngineResult<Self> {
        let host = cpal::default_host();

        let device = host.default_output_device().ok_or_else(|| {
            EngineError::from_detail(ErrorCode::NotFound, "No audio output device found")
        })?;

        info!("Audio output device: {:?}", device.name());

        let config = device.default_output_config().map_err(|e| {
            EngineError::from_detail(
                ErrorCode::Internal,
                format!("Failed to get default output config: {}", e),
            )
        })?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as u32;

        info!(
            "Audio output config: sample_rate={}, channels={}, format={:?}",
            sample_rate,
            channels,
            config.sample_format()
        );

        let ring_size = RING_BUFFER_FRAMES * channels as usize;
        let ring = HeapRb::<f32>::new(ring_size);
        let (producer, consumer) = ring.split();

        let sync = AudioSync::new();
        let low_watermark_samples = LOW_WATERMARK_FRAMES * channels as usize;
        let high_watermark_samples = HIGH_WATERMARK_FRAMES * channels as usize;
        let stream_error = Arc::new(AtomicBool::new(false));

        let stream = match config.sample_format() {
            SampleFormat::F32 => build_stream_f32(
                &device,
                &config.into(),
                consumer,
                sync.clone(),
                low_watermark_samples,
                stream_error.clone(),
            )?,
            SampleFormat::I16 => build_stream_converted::<i16>(
                &device,
                &config.into(),
                consumer,
                sync.clone(),
                low_watermark_samples,
                stream_error.clone(),
            )?,
            SampleFormat::U16 => build_stream_converted::<u16>(
                &device,
                &config.into(),
                consumer,
                sync.clone(),
                low_watermark_samples,
                stream_error.clone(),
            )?,
            format => {
                return Err(EngineError::from_detail(
                    ErrorCode::Unsupported,
                    format!("Unsupported sample format: {:?}", format),
                ));
            }
        };

        stream.play().map_err(|e| {
            EngineError::from_detail(
                ErrorCode::Internal,
                format!("Failed to start audio stream: {}", e),
            )
        })?;

        Ok(Self {
            stream,
            producer,
            sample_rate,
            channels,
            sync,
            low_watermark_samples,
            high_watermark_samples,
            stream_error,
        })
    }

    /// Get the sample rate
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the number of channels
    #[inline]
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Write samples to the output buffer
    /// Returns the number of samples written
    #[inline]
    pub fn write(&mut self, samples: &[f32]) -> usize {
        self.producer.push_slice(samples)
    }

    /// Get available space in the buffer (in samples)
    #[inline]
    pub fn available(&self) -> usize {
        self.producer.vacant_len()
    }

    /// Currently buffered sample count, read from the producer side so it is
    /// always fresh — unlike the callback-updated `buffer_level` hint behind
    /// [`needs_data`](Self::needs_data). Use this to decide how much to refill.
    #[inline]
    pub fn buffered(&self) -> usize {
        self.producer.occupied_len()
    }

    /// Refill target depth in samples: fill up to here, not the whole ring.
    ///
    /// Never below twice the largest observed device callback, so a device that
    /// requests large blocks can always hold a full callback (otherwise every
    /// callback would partially underrun). The fill loop's `available() >=
    /// buffer_size` check still bounds this to the ring capacity.
    #[inline]
    pub fn high_watermark(&self) -> usize {
        self.high_watermark_samples
            .max(self.sync.max_callback().saturating_mul(2))
    }

    /// Check if buffer needs more data
    #[inline]
    pub fn needs_data(&self) -> bool {
        self.sync.buffer_level() < self.low_watermark_samples
    }

    /// Get sync handle for checking callback signals
    #[inline]
    pub fn sync(&self) -> &AudioSync {
        &self.sync
    }

    /// Check if the audio stream is still alive (no errors reported).
    #[inline]
    pub fn is_alive(&self) -> bool {
        !self.stream_error.load(Ordering::Acquire)
    }

    /// Pause the audio stream (stops the hardware callback).
    /// Use when the app is backgrounded or no audio is playing to save power.
    pub fn pause_stream(&self) -> bool {
        match self.stream.pause() {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("Failed to pause audio stream: {}", e);
                false
            }
        }
    }

    /// Resume the audio stream (restarts the hardware callback).
    pub fn resume_stream(&self) -> bool {
        match self.stream.play() {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("Failed to resume audio stream: {}", e);
                self.stream_error.store(true, Ordering::Release);
                false
            }
        }
    }
}

/// Samples converted per pass through the stack scratch buffer.
///
/// 512 f32 is 2 KiB of stack, which every real-time audio callback thread has
/// (AAudio, ALSA and CoreAudio all give their callback threads a stack measured
/// in tens of kilobytes). Chunking is what removes the heap from the callback:
/// the scratch is sized by this constant rather than by whatever the device asks
/// for, so no device buffer size can make the callback allocate.
const CONVERT_CHUNK_SAMPLES: usize = 512;

/// A device sample format the ring's `f32` can be written as.
///
/// A trait rather than three copies of the callback: the conversion is the only
/// difference between the integer formats, and it monomorphises away.
trait FromNormalizedF32: Copy {
    fn from_normalized(sample: f32) -> Self;
}

impl FromNormalizedF32 for i16 {
    #[inline]
    fn from_normalized(sample: f32) -> Self {
        (sample.clamp(-1.0, 1.0) * 32767.0) as Self
    }
}

impl FromNormalizedF32 for u16 {
    #[inline]
    fn from_normalized(sample: f32) -> Self {
        // Centred at 32768, matching the unsigned PCM convention.
        ((sample.clamp(-1.0, 1.0) * 32767.0) + 32768.0) as Self
    }
}

/// Everything the hardware callback does, as a named type rather than a closure.
///
/// **This exists so the real-time region can be measured.** The callback runs on
/// a thread the platform schedules as real-time -- `SCHED_FIFO` on Android -- and
/// an allocation there is not slow, it is a missed deadline heard as a dropout,
/// because the allocator can block behind a thread that is not real-time
/// scheduled at all. Section 7.3 requires that to be enforced by a test, and a
/// closure handed to `build_output_stream` cannot be reached without a device.
///
/// The type is also what removes the duplication: three near-identical callbacks
/// differing only in the sample conversion collapse into two methods.
struct OutputCallback {
    consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
}

impl OutputCallback {
    /// The device speaks the ring's own format: pop straight into its buffer.
    fn render_native(&mut self, data: &mut [f32]) {
        self.sync.observe_callback(data.len());

        let read = self.consumer.pop_slice(data);
        if read < data.len() {
            data[read..].fill(0.0);
        }

        self.report_depth();
    }

    /// The device speaks an integer format: convert through a fixed stack
    /// scratch, a chunk at a time.
    ///
    /// The scratch used to be a `Vec<f32>` pre-sized to 4096 samples and grown
    /// with `resize` whenever the device asked for more. A device is under no
    /// obligation to request the same number of samples every callback, or a
    /// number this code guessed -- AAudio's `numFrames` varies across route
    /// changes and stream recovery, and cpal's ALSA backend sizes each callback
    /// from the period space actually available -- so that `resize` was a
    /// `realloc` inside a real-time callback. A fixed scratch cannot be grown,
    /// so the failure mode is gone rather than made unlikely.
    fn render_converted<T: FromNormalizedF32>(&mut self, data: &mut [T]) {
        self.sync.observe_callback(data.len());

        let mut scratch = [0.0f32; CONVERT_CHUNK_SAMPLES];
        for out in data.chunks_mut(CONVERT_CHUNK_SAMPLES) {
            let scratch = &mut scratch[..out.len()];
            let read = self.consumer.pop_slice(scratch);
            if read < scratch.len() {
                scratch[read..].fill(0.0);
            }
            for (dst, &sample) in out.iter_mut().zip(scratch.iter()) {
                *dst = T::from_normalized(sample);
            }
        }

        self.report_depth();
    }

    /// Publish the post-callback ring depth and ask for a refill if it is low.
    #[inline]
    fn report_depth(&self) {
        let remaining = self.consumer.occupied_len();
        self.sync.update_level(remaining);

        if remaining < self.low_watermark {
            self.sync.signal_need_data();
        }
    }
}

fn build_stream_f32(
    device: &Device,
    config: &StreamConfig,
    consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
    stream_error: Arc<AtomicBool>,
) -> EngineResult<Stream> {
    let mut callback = OutputCallback {
        consumer,
        sync,
        low_watermark,
    };
    build_stream(device, config, stream_error, move |data: &mut [f32]| {
        callback.render_native(data)
    })
}

fn build_stream_converted<T>(
    device: &Device,
    config: &StreamConfig,
    consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
    stream_error: Arc<AtomicBool>,
) -> EngineResult<Stream>
where
    T: cpal::SizedSample + FromNormalizedF32 + 'static,
{
    let mut callback = OutputCallback {
        consumer,
        sync,
        low_watermark,
    };
    build_stream(device, config, stream_error, move |data: &mut [T]| {
        callback.render_converted(data)
    })
}

fn build_stream<T, F>(
    device: &Device,
    config: &StreamConfig,
    stream_error: Arc<AtomicBool>,
    mut render: F,
) -> EngineResult<Stream>
where
    T: cpal::SizedSample + 'static,
    F: FnMut(&mut [T]) + Send + 'static,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| render(data),
            move |err| {
                error!("Audio output error: {}", err);
                stream_error.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| {
            EngineError::from_detail(
                ErrorCode::Internal,
                format!("Failed to build audio stream: {}", e),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use migo_alloc_probe::{Burst, assert_no_steady_state_allocation};

    const WARMUP: usize = 8;
    const MEASURED: usize = 64;

    /// Deliberately larger than the 4096-sample conversion buffer this callback
    /// used to pre-size, so a device that asks for more than the code guessed is
    /// what the gates run against.
    const CALLBACK_SAMPLES: usize = 8192;

    /// A callback wired to its own ring, filled, with the producer kept alive.
    ///
    /// The producer is returned rather than dropped because a dropped producer
    /// closes the ring, and a gate that measured a closed ring would be measuring
    /// the wrong thing.
    fn filled_callback(ring_samples: usize) -> (HeapProd<f32>, OutputCallback) {
        let (mut producer, consumer) = HeapRb::<f32>::new(ring_samples).split();
        let block = vec![0.25f32; ring_samples];
        producer.push_slice(&block);
        (
            producer,
            OutputCallback {
                consumer,
                sync: AudioSync::new(),
                low_watermark: ring_samples / 4,
            },
        )
    }

    /// Section 7.3's steady-state allocation gate, on the hardware callback.
    ///
    /// One iteration is one device callback plus the refill that keeps the ring
    /// from draining, which is the pair that actually repeats forever. Both
    /// device shapes are covered in the same burst: the native `f32` path pops
    /// straight into the device buffer, the integer path converts, and only the
    /// second one ever owned a buffer.
    #[test]
    fn a_steady_state_output_callback_never_reaches_the_heap() {
        let ring_samples = CALLBACK_SAMPLES * 2;
        let (mut native_producer, mut native) = filled_callback(ring_samples);
        let (mut converted_producer, mut converted) = filled_callback(ring_samples);

        // The reservoir is built before the measured window, per Section 7.3's
        // rule that a burst body must not take from a pool it does not control.
        let refill = vec![0.25f32; CALLBACK_SAMPLES];
        let mut native_out = vec![0.0f32; CALLBACK_SAMPLES];
        let mut converted_out = vec![0i16; CALLBACK_SAMPLES];

        assert_no_steady_state_allocation(
            Burst {
                path: "audio: one hardware output callback, native and converted",
                warmup: WARMUP,
                measured: MEASURED,
            },
            |_| {
                native.render_native(&mut native_out);
                native_producer.push_slice(&refill);

                converted.render_converted(&mut converted_out);
                converted_producer.push_slice(&refill);
            },
        );
    }

    /// The same property, but from cold — which is the one that was broken.
    ///
    /// A steady-state burst cannot see this defect, and saying why is the point:
    /// the conversion buffer only ever grew, so the one `realloc` a device buffer
    /// larger than 4096 samples caused happened during the warm-up and the
    /// measured window was clean. That is not an acceptable answer here. This
    /// callback runs on a real-time-scheduled thread, where the first call is as
    /// deadline-bound as the ten-thousandth, and stream recovery and route
    /// changes rebuild it with a buffer size nobody chose.
    ///
    /// So every iteration gets a callback that has never run, all of them built
    /// before the measured window. That needs no new mechanism: a burst over a
    /// fleet of one-shot callbacks measures first calls exactly.
    #[test]
    fn an_output_callback_never_reaches_the_heap_on_its_very_first_call() {
        let mut fleet: Vec<(HeapProd<f32>, OutputCallback)> = (0..WARMUP + MEASURED)
            .map(|_| filled_callback(CALLBACK_SAMPLES))
            .collect();
        let mut out = vec![0i16; CALLBACK_SAMPLES];

        assert_no_steady_state_allocation(
            Burst {
                path: "audio: a hardware output callback's first call, at a device buffer size \
                       larger than any the callback pre-sized for",
                warmup: WARMUP,
                measured: MEASURED,
            },
            |iteration| {
                fleet[iteration].1.render_converted(&mut out);
            },
        );
    }

    /// The conversion must still be the conversion. A chunked scratch that
    /// silenced or reordered samples would pass both gates above and be inaudible
    /// to them, so the boundary between chunks is asserted directly.
    #[test]
    fn chunked_conversion_matches_the_ring_across_chunk_boundaries() {
        // Not a multiple of the chunk size, so the final partial chunk is covered.
        let samples = CONVERT_CHUNK_SAMPLES * 2 + 7;
        let source: Vec<f32> = (0..samples)
            .map(|i| (i as f32 / samples as f32) * 2.0 - 1.0)
            .collect();

        let (mut producer, consumer) = HeapRb::<f32>::new(samples).split();
        producer.push_slice(&source);
        let mut callback = OutputCallback {
            consumer,
            sync: AudioSync::new(),
            low_watermark: 0,
        };

        let mut out = vec![0i16; samples];
        callback.render_converted(&mut out);

        for (index, (&want, &got)) in source.iter().zip(out.iter()).enumerate() {
            assert_eq!(
                got,
                i16::from_normalized(want),
                "sample {index} converted wrong across a chunk boundary"
            );
        }
    }

    /// A callback handed more than the ring holds must pad with silence rather
    /// than leave the device buffer at whatever it contained.
    #[test]
    fn an_underrun_pads_the_rest_of_the_device_buffer_with_silence() {
        let available = CONVERT_CHUNK_SAMPLES + 3;
        let (mut producer, consumer) = HeapRb::<f32>::new(available).split();
        producer.push_slice(&vec![1.0f32; available]);
        let mut callback = OutputCallback {
            consumer,
            sync: AudioSync::new(),
            low_watermark: usize::MAX,
        };

        let mut out = vec![0xAAu16.wrapping_mul(3); available * 2];
        callback.render_converted(&mut out);

        assert!(
            out[..available]
                .iter()
                .all(|&s| s == u16::from_normalized(1.0)),
            "the buffered samples must reach the device buffer"
        );
        assert!(
            out[available..]
                .iter()
                .all(|&s| s == u16::from_normalized(0.0)),
            "an underrun must pad with silence, not leave stale device samples"
        );
        assert!(
            callback.sync.check_and_clear(),
            "an underrun must ask the audio thread for a refill"
        );
    }
}
