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

/// Lightweight signaling for callback-driven audio
/// Uses atomic flag instead of Condvar for lower overhead
#[derive(Clone)]
pub struct AudioSync {
    /// Flag indicating buffer needs data
    needs_data: Arc<AtomicBool>,
    /// Current buffer fill level (in samples)
    buffer_level: Arc<AtomicUsize>,
}

impl AudioSync {
    fn new() -> Self {
        Self {
            needs_data: Arc::new(AtomicBool::new(false)),
            buffer_level: Arc::new(AtomicUsize::new(0)),
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
}

/// Audio output handle
pub struct AudioOutput {
    _stream: Stream,
    producer: HeapProd<f32>,
    sample_rate: u32,
    channels: u32,
    sync: AudioSync,
    low_watermark_samples: usize,
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
            SampleFormat::I16 => build_stream_i16(
                &device,
                &config.into(),
                consumer,
                sync.clone(),
                low_watermark_samples,
                stream_error.clone(),
            )?,
            SampleFormat::U16 => build_stream_u16(
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
            _stream: stream,
            producer,
            sample_rate,
            channels,
            sync,
            low_watermark_samples,
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
}

// Separate functions for each sample format to avoid generic trait object overhead
// and to pre-allocate the conversion buffer

fn build_stream_f32(
    device: &Device,
    config: &StreamConfig,
    mut consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
    stream_error: Arc<AtomicBool>,
) -> EngineResult<Stream> {
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Direct read into output buffer - no allocation!
                let read = consumer.pop_slice(data);

                // Fill remaining with silence
                if read < data.len() {
                    data[read..].fill(0.0);
                }

                // Update level and signal if needed
                let remaining = consumer.occupied_len();
                sync.update_level(remaining);

                if remaining < low_watermark {
                    sync.signal_need_data();
                }
            },
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
        })?;

    Ok(stream)
}

fn build_stream_i16(
    device: &Device,
    config: &StreamConfig,
    mut consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
    stream_error: Arc<AtomicBool>,
) -> EngineResult<Stream> {
    // Pre-allocate conversion buffer based on typical callback size
    // Will be resized if needed (rare)
    let mut temp_buffer: Vec<f32> = Vec::with_capacity(4096);

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                // Resize temp buffer if needed (should be rare after warmup)
                if temp_buffer.len() < data.len() {
                    temp_buffer.resize(data.len(), 0.0);
                }

                let temp = &mut temp_buffer[..data.len()];
                let read = consumer.pop_slice(temp);

                // Fill remaining with silence
                if read < temp.len() {
                    temp[read..].fill(0.0);
                }

                // Convert f32 to i16
                for (i, &sample) in temp.iter().enumerate() {
                    data[i] = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                }

                let remaining = consumer.occupied_len();
                sync.update_level(remaining);

                if remaining < low_watermark {
                    sync.signal_need_data();
                }
            },
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
        })?;

    Ok(stream)
}

fn build_stream_u16(
    device: &Device,
    config: &StreamConfig,
    mut consumer: HeapCons<f32>,
    sync: AudioSync,
    low_watermark: usize,
    stream_error: Arc<AtomicBool>,
) -> EngineResult<Stream> {
    let mut temp_buffer: Vec<f32> = Vec::with_capacity(4096);

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                if temp_buffer.len() < data.len() {
                    temp_buffer.resize(data.len(), 0.0);
                }

                let temp = &mut temp_buffer[..data.len()];
                let read = consumer.pop_slice(temp);

                if read < temp.len() {
                    temp[read..].fill(0.0);
                }

                // Convert f32 to u16 (center at 32768)
                for (i, &sample) in temp.iter().enumerate() {
                    data[i] = ((sample.clamp(-1.0, 1.0) * 32767.0) + 32768.0) as u16;
                }

                let remaining = consumer.occupied_len();
                sync.update_level(remaining);

                if remaining < low_watermark {
                    sync.signal_need_data();
                }
            },
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
        })?;

    Ok(stream)
}
