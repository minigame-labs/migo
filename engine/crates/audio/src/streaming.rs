//! Streaming audio download and decode for edge-download-edge-play
//!
//! Downloads audio from URL in chunks and decodes progressively.
//! Only MP3 is supported for streaming as it can be decoded frame-by-frame.
//!
//! Uses a shared tokio runtime for all download tasks to avoid per-stream overhead.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use shared::error::{EngineError, EngineResult, ErrorCode};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Message from download task to audio thread
pub enum StreamMsg {
    /// Audio format detected, can start playback
    Ready { sample_rate: u32, channels: u32 },
    /// New decoded samples available
    Samples(Vec<f32>),
    /// Download/decode complete
    Done,
    /// Error occurred
    Error(String),
}

/// Shared state for streaming progress
pub struct StreamingState {
    /// Total bytes downloaded
    pub bytes_downloaded: AtomicU64,
    /// Total bytes expected (0 if unknown)
    pub bytes_total: AtomicU64,
    /// Whether download is complete
    pub download_complete: AtomicBool,
    /// Whether cancelled
    pub cancelled: AtomicBool,
}

impl StreamingState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            bytes_downloaded: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            download_complete: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for StreamingState {
    fn default() -> Self {
        Self {
            bytes_downloaded: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            download_complete: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }
}

/// Shared runtime for all audio streaming downloads
/// Uses a multi-threaded runtime with 2 worker threads
static STREAM_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn get_stream_runtime() -> &'static tokio::runtime::Runtime {
    STREAM_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("audio-stream")
            .enable_all()
            .build()
            .expect("Failed to create audio streaming runtime")
    })
}

/// Start streaming download and decode
/// Returns a receiver for decoded audio chunks
pub fn start_streaming_download(
    url: String,
    state: Arc<StreamingState>,
    target_sample_rate: u32,
) -> mpsc::UnboundedReceiver<StreamMsg> {
    let (tx, rx) = mpsc::unbounded_channel();

    // Spawn download task on the shared runtime
    get_stream_runtime().spawn(async move {
        if let Err(e) = streaming_download_task(url, tx.clone(), state, target_sample_rate).await {
            let _ = tx.send(StreamMsg::Error(e.to_string()));
        }
    });

    rx
}

async fn streaming_download_task(
    url: String,
    tx: mpsc::UnboundedSender<StreamMsg>,
    state: Arc<StreamingState>,
    target_sample_rate: u32,
) -> EngineResult<()> {
    debug!("Starting streaming download: {}", url);

    let client = reqwest::Client::new();
    let response = client.get(&url).send().await.map_err(|e| {
        tracing::error!("HTTP request failed for {}: {}", url, e);
        EngineError::from_detail(ErrorCode::IoError, format!("HTTP request failed: {}", e))
    })?;

    if !response.status().is_success() {
        tracing::error!("HTTP error for {}: {}", url, response.status());
        return Err(EngineError::from_detail(
            ErrorCode::IoError,
            format!("HTTP error: {}", response.status()),
        ));
    }

    // Get content length if available
    if let Some(len) = response.content_length() {
        state.bytes_total.store(len, Ordering::Relaxed);
        tracing::trace!("Content-Length: {} bytes", len);
    } else {
        tracing::trace!("Content-Length not available (chunked transfer)");
    }

    // Stream the response body
    let mut stream = response.bytes_stream();
    let mut decoder = Mp3StreamDecoder::new(target_sample_rate);
    let mut ready_sent = false;

    use futures_util::StreamExt;

    let mut chunk_count = 0u32;
    let mut total_decoded_samples = 0usize;

    while let Some(chunk_result) = stream.next().await {
        // Check for cancellation
        if state.is_cancelled() {
            debug!("Streaming download cancelled");
            return Ok(());
        }

        let chunk = chunk_result.map_err(|e| {
            tracing::error!("Stream chunk error: {}", e);
            EngineError::from_detail(ErrorCode::IoError, format!("Stream error: {}", e))
        })?;

        chunk_count += 1;
        let chunk_len = chunk.len();

        // Update progress
        let downloaded = state
            .bytes_downloaded
            .fetch_add(chunk_len as u64, Ordering::Relaxed)
            + chunk_len as u64;

        // Log progress every 10 chunks or every 100KB
        if chunk_count % 10 == 0 || chunk_count == 1 {
            let total = state.bytes_total.load(Ordering::Relaxed);
            if total > 0 {
                let percent = (downloaded as f64 / total as f64 * 100.0) as u32;
                tracing::trace!(
                    "Download progress: {}% ({}/{} bytes, {} chunks)",
                    percent,
                    downloaded,
                    total,
                    chunk_count
                );
            } else {
                tracing::trace!(
                    "Download progress: {} bytes, {} chunks",
                    downloaded,
                    chunk_count
                );
            }
        }

        // Feed chunk to decoder
        decoder.push_data(&chunk);

        // Try to decode frames
        let (new_samples, sample_rate, channels) = decoder.decode_available();

        // Send ready message once we have format info
        if !ready_sent && sample_rate > 0 && channels > 0 {
            tracing::debug!(
                "Stream format detected: {} Hz, {} channels",
                sample_rate,
                channels
            );
            let _ = tx.send(StreamMsg::Ready {
                sample_rate,
                channels,
            });
            ready_sent = true;
        }

        // Send decoded samples
        if !new_samples.is_empty() {
            total_decoded_samples += new_samples.len();
            tracing::trace!(
                "Decoded {} samples (total: {})",
                new_samples.len(),
                total_decoded_samples
            );
            let _ = tx.send(StreamMsg::Samples(new_samples));
        }
    }

    // Flush remaining data
    let (final_samples, _, _) = decoder.flush();
    if !final_samples.is_empty() {
        total_decoded_samples += final_samples.len();
        tracing::trace!("Flushed {} final samples", final_samples.len());
        let _ = tx.send(StreamMsg::Samples(final_samples));
    }

    state.download_complete.store(true, Ordering::Release);
    let _ = tx.send(StreamMsg::Done);

    let downloaded = state.bytes_downloaded.load(Ordering::Relaxed);
    debug!(
        "Streaming download complete: {} bytes, {} total samples decoded",
        downloaded, total_decoded_samples
    );
    Ok(())
}

/// MP3 streaming decoder
struct Mp3StreamDecoder {
    /// Accumulated raw data
    buffer: Vec<u8>,
    /// Position of last successful decode
    decode_pos: usize,
    /// Detected sample rate
    sample_rate: u32,
    /// Detected channels
    channels: u32,
    /// Target sample rate for resampling
    target_sample_rate: u32,
    /// Resampler state (if needed)
    resampler: Option<crate::resampler::StreamResampler>,
}

impl Mp3StreamDecoder {
    fn new(target_sample_rate: u32) -> Self {
        Self {
            buffer: Vec::with_capacity(128 * 1024),
            decode_pos: 0,
            sample_rate: 0,
            channels: 0,
            target_sample_rate,
            resampler: None,
        }
    }

    fn push_data(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Decode available frames, returns (samples, sample_rate, channels)
    fn decode_available(&mut self) -> (Vec<f32>, u32, u32) {
        let mut samples = Vec::new();

        // Create decoder from current buffer position
        let data_to_decode = &self.buffer[self.decode_pos..];
        if data_to_decode.is_empty() {
            return (samples, self.sample_rate, self.channels);
        }

        let mut decoder = minimp3::Decoder::new(std::io::Cursor::new(data_to_decode));
        let mut bytes_consumed = 0;

        loop {
            match decoder.next_frame() {
                Ok(frame) => {
                    // First frame - detect format
                    if self.sample_rate == 0 {
                        self.sample_rate = frame.sample_rate as u32;
                        self.channels = frame.channels as u32;

                        // Create resampler if needed
                        if self.sample_rate != self.target_sample_rate {
                            self.resampler = Some(crate::resampler::StreamResampler::new(
                                self.sample_rate,
                                self.target_sample_rate,
                                self.channels,
                            ));
                        }
                    }

                    // Convert i16 to f32
                    let frame_samples: Vec<f32> =
                        frame.data.iter().map(|&s| s as f32 / 32768.0).collect();

                    // Resample if needed
                    let output_samples = if let Some(ref mut resampler) = self.resampler {
                        resampler.process(&frame_samples)
                    } else {
                        frame_samples
                    };

                    samples.extend(output_samples);

                    // Get exact bytes consumed from cursor position
                    bytes_consumed = decoder.reader().position() as usize;
                }
                Err(minimp3::Error::Eof) => break,
                Err(minimp3::Error::InsufficientData) => break,
                Err(e) => {
                    warn!("MP3 decode error: {:?}, skipping", e);
                    // Try to skip bad data
                    bytes_consumed += 1;
                    break;
                }
            }
        }

        // Update decode position
        self.decode_pos += bytes_consumed;

        // Trim buffer if it gets too large (keep last 4KB for potential incomplete frame)
        if self.decode_pos > 64 * 1024 {
            let keep_from = self.decode_pos.saturating_sub(4096);
            self.buffer.drain(..keep_from);
            self.decode_pos -= keep_from;
        }

        (samples, self.sample_rate, self.channels)
    }

    /// Flush remaining data
    fn flush(&mut self) -> (Vec<f32>, u32, u32) {
        // Try one more decode pass
        self.decode_available()
    }
}
