//! Streaming audio download and decode for edge-download-edge-play
//!
//! Downloads audio from URL in chunks and decodes progressively.
//! Only MP3 is supported for streaming as it can be decoded frame-by-frame.
//!
//! Uses a shared tokio runtime for all download tasks to avoid per-stream overhead.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use shared::error::{EngineError, EngineResult, ErrorCode};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, warn};

use crate::off_worker::OffWorker;

/// Keep only a small number of decoded chunks ahead of the audio thread.
/// When the app is backgrounded and stops polling, async sends apply
/// backpressure to both decoding and the network response body.
const STREAM_CHANNEL_CAPACITY: usize = 4;

/// Bound a server that accepts a connection but never produces headers.
const RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-chunk inactivity bound. This deliberately does not cap track duration.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-host factory captured by `AudioService`. Building remains lazy so local
/// audio and hosts that never stream do not allocate a reqwest connection pool.
pub type StreamingHttpClientFactory =
    Arc<dyn Fn() -> EngineResult<reqwest::Client> + Send + Sync + 'static>;

/// One lazily-built reqwest client for one host audio thread. `Client::clone`
/// shares reqwest's internal pool and is the only value moved into each task.
pub(crate) struct LazyStreamingClient {
    factory: StreamingHttpClientFactory,
    client: Option<reqwest::Client>,
}

impl LazyStreamingClient {
    pub(crate) fn new(factory: StreamingHttpClientFactory) -> Self {
        Self {
            factory,
            client: None,
        }
    }

    pub(crate) fn get(&mut self) -> EngineResult<reqwest::Client> {
        if let Some(client) = &self.client {
            return Ok(client.clone());
        }
        let client = (self.factory)()?;
        self.client = Some(client.clone());
        Ok(client)
    }
}

fn stream_channel() -> (mpsc::Sender<StreamMsg>, mpsc::Receiver<StreamMsg>) {
    mpsc::channel(STREAM_CHANNEL_CAPACITY)
}

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
    /// Event-driven cancellation edge for tasks blocked on network I/O.
    cancel_notify: Notify,
}

impl StreamingState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            bytes_downloaded: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            download_complete: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            cancel_notify: Notify::new(),
        })
    }

    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.cancel_notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Wait without polling. Registering before the second level check closes
    /// the check-to-sleep race with `notify_waiters`, which does not retain a
    /// permit for future waiters.
    async fn cancelled(&self) {
        loop {
            let notified = self.cancel_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for StreamingState {
    fn default() -> Self {
        Self {
            bytes_downloaded: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            download_complete: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            cancel_notify: Notify::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NetworkWait<T> {
    Ready(T),
    Cancelled,
    TimedOut,
}

/// Race one network await against the event-driven cancellation edge and an
/// inactivity deadline. Generic/monomorphized so the production path allocates
/// no boxed future or polling task.
async fn wait_for_network<T, F>(
    state: &StreamingState,
    timeout: Duration,
    operation: F,
) -> NetworkWait<T>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = state.cancelled() => NetworkWait::Cancelled,
        result = tokio::time::timeout(timeout, operation) => match result {
            Ok(value) => NetworkWait::Ready(value),
            Err(_) => NetworkWait::TimedOut,
        },
    }
}

/// Shared single-worker runtime for all audio streaming downloads.
static STREAM_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn get_stream_runtime() -> &'static tokio::runtime::Runtime {
    STREAM_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("audio-stream")
            .enable_all()
            .build()
            .expect("Failed to create audio streaming runtime")
    })
}

/// Start streaming download and decode
/// Returns a receiver for decoded audio chunks
pub fn start_streaming_download(
    client: reqwest::Client,
    url: String,
    state: Arc<StreamingState>,
    target_sample_rate: u32,
) -> mpsc::Receiver<StreamMsg> {
    let (tx, rx) = stream_channel();

    // Spawn download task on the shared runtime
    get_stream_runtime().spawn(async move {
        if let Err(e) =
            streaming_download_task(client, url, tx.clone(), state, target_sample_rate).await
        {
            let _ = tx.send(StreamMsg::Error(e.to_string())).await;
        }
    });

    rx
}

async fn streaming_download_task(
    client: reqwest::Client,
    url: String,
    tx: mpsc::Sender<StreamMsg>,
    state: Arc<StreamingState>,
    target_sample_rate: u32,
) -> EngineResult<()> {
    debug!("Starting streaming download: {}", url);

    if state.is_cancelled() {
        return Ok(());
    }

    let response =
        match wait_for_network(&state, RESPONSE_HEADER_TIMEOUT, client.get(&url).send()).await {
            NetworkWait::Cancelled => return Ok(()),
            NetworkWait::TimedOut => {
                return Err(EngineError::from_detail(
                    ErrorCode::Timeout,
                    "audio response header timeout",
                ));
            }
            NetworkWait::Ready(Ok(response)) => response,
            NetworkWait::Ready(Err(error)) => {
                tracing::error!("HTTP request failed for {}: {}", url, error);
                return Err(EngineError::from_detail(
                    ErrorCode::IoError,
                    format!("HTTP request failed: {error}"),
                ));
            }
        };

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
    let mut decoder = OffWorker::new(Mp3StreamDecoder::new(target_sample_rate));
    let mut ready_sent = false;

    use futures_util::StreamExt;

    let mut chunk_count = 0u32;
    let mut total_decoded_samples = 0usize;

    loop {
        let next_chunk = match wait_for_network(&state, BODY_IDLE_TIMEOUT, stream.next()).await {
            NetworkWait::Cancelled => {
                debug!("Streaming download cancelled");
                return Ok(());
            }
            NetworkWait::TimedOut => {
                return Err(EngineError::from_detail(
                    ErrorCode::Timeout,
                    "audio response body idle timeout",
                ));
            }
            NetworkWait::Ready(chunk) => chunk,
        };

        let Some(chunk_result) = next_chunk else {
            break;
        };

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

        // Feed the chunk to the decoder and take whatever frames it completed. Both
        // are CPU-bound and this task's worker is shared with every other session's
        // download, so the step runs where the worker is not waiting for it.
        let (rest, (new_samples, sample_rate, channels)) = decoder
            .with(move |decoder| {
                decoder.push_data(&chunk);
                decoder.decode_available()
            })
            .await?;
        decoder = rest;

        // Send ready message once we have format info
        if !ready_sent && sample_rate > 0 && channels > 0 {
            tracing::debug!(
                "Stream format detected: {} Hz, {} channels",
                sample_rate,
                channels
            );
            if tx
                .send(StreamMsg::Ready {
                    sample_rate,
                    channels,
                })
                .await
                .is_err()
            {
                return Ok(());
            }
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
            if tx.send(StreamMsg::Samples(new_samples)).await.is_err() {
                return Ok(());
            }
        }
    }

    // Flush remaining data
    let (_decoder, (final_samples, _, _)) = decoder.with(Mp3StreamDecoder::flush).await?;
    if !final_samples.is_empty() {
        total_decoded_samples += final_samples.len();
        tracing::trace!("Flushed {} final samples", final_samples.len());
        if tx.send(StreamMsg::Samples(final_samples)).await.is_err() {
            return Ok(());
        }
    }

    state.download_complete.store(true, Ordering::Release);
    if tx.send(StreamMsg::Done).await.is_err() {
        return Ok(());
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use migo_executor_probe::{PATIENCE, SharedExecutor, assert_leaves_the_executor_free};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use tokio::sync::mpsc::error::TrySendError;

    /// Not MP3, and deliberately so: the decoder keeps what it cannot yet decode, so
    /// the buffered length is a value only the real decoder produces.
    const A_CHUNK: &[u8] = b"bytes no frame ends in";

    #[test]
    fn the_decode_step_leaves_the_shared_streaming_worker_free() {
        // `STREAM_RUNTIME` is process-wide and single-worker, so a decode that ran on
        // it would stall every other session's download for as long as the decode
        // took -- which is as long as the chunk is, not as long as anyone budgeted.
        let buffered = assert_leaves_the_executor_free(
            SharedExecutor {
                step: "streaming MP3 decode",
                executor: "the process-wide audio streaming worker",
                patience: PATIENCE,
            },
            get_stream_runtime(),
            |cpu| async move {
                let (_decoder, buffered) = OffWorker::new(Mp3StreamDecoder::new(48_000))
                    .with(move |decoder| {
                        cpu.occupy();
                        decoder.push_data(A_CHUNK);
                        decoder.decode_available();
                        decoder.buffer.len()
                    })
                    .await
                    .expect("the decode step must complete");
                buffered
            },
        );

        assert_eq!(
            buffered,
            A_CHUNK.len(),
            "the step must have run against the real decoder, not a stand-in"
        );
    }

    #[test]
    fn stream_channel_applies_backpressure_at_capacity() {
        let (tx, mut rx) = stream_channel();

        for _ in 0..STREAM_CHANNEL_CAPACITY {
            assert!(tx.try_send(StreamMsg::Done).is_ok());
        }
        assert!(matches!(
            tx.try_send(StreamMsg::Done),
            Err(TrySendError::Full(_))
        ));

        assert!(rx.try_recv().is_ok());
        assert!(tx.try_send(StreamMsg::Done).is_ok());
    }

    fn counting_factory(calls: Arc<AtomicUsize>) -> StreamingHttpClientFactory {
        Arc::new(move || {
            calls.fetch_add(1, AtomicOrdering::Relaxed);
            reqwest::Client::builder().build().map_err(|error| {
                EngineError::from_detail(
                    ErrorCode::IoError,
                    format!("test client build failed: {error}"),
                )
            })
        })
    }

    #[test]
    fn streaming_client_is_lazy_and_built_once_for_multiple_tracks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut client = LazyStreamingClient::new(counting_factory(calls.clone()));

        assert_eq!(calls.load(AtomicOrdering::Relaxed), 0);
        let _first = client.get().expect("first client");
        let _second = client.get().expect("reused client");
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn failed_client_build_is_not_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_factory = calls.clone();
        let factory: StreamingHttpClientFactory = Arc::new(move || {
            calls_for_factory.fetch_add(1, AtomicOrdering::Relaxed);
            Err(EngineError::from_detail(
                ErrorCode::IoError,
                "injected client build failure",
            ))
        });
        let mut client = LazyStreamingClient::new(factory);

        assert!(client.get().is_err());
        assert!(client.get().is_err());
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 2);
    }

    #[test]
    fn streaming_clients_are_isolated_per_host_instance() {
        let host_a_calls = Arc::new(AtomicUsize::new(0));
        let host_b_calls = Arc::new(AtomicUsize::new(0));
        let mut host_a = LazyStreamingClient::new(counting_factory(host_a_calls.clone()));
        let mut host_b = LazyStreamingClient::new(counting_factory(host_b_calls.clone()));

        let _ = host_a.get().unwrap();
        let _ = host_a.get().unwrap();
        let _ = host_b.get().unwrap();
        assert_eq!(host_a_calls.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(host_b_calls.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn cancellation_waiter_handles_pre_cancel_and_in_flight_cancel() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let pre_cancelled = StreamingState::new();
            pre_cancelled.cancel();
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                pre_cancelled.cancelled(),
            )
            .await
            .expect("pre-cancelled waiter must resolve");

            let in_flight = StreamingState::new();
            let waiter = in_flight.cancelled();
            tokio::pin!(waiter);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiter)
                    .await
                    .is_err(),
                "waiter must remain pending before cancellation"
            );
            in_flight.cancel();
            tokio::time::timeout(std::time::Duration::from_millis(50), waiter)
                .await
                .expect("registered waiter must observe cancellation");
        });
    }

    #[test]
    fn network_wait_covers_ready_timeout_and_registered_cancellation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let active = StreamingState::new();
            assert_eq!(
                wait_for_network(&active, Duration::from_millis(50), std::future::ready(7u8),)
                    .await,
                NetworkWait::Ready(7)
            );
            assert_eq!(
                wait_for_network(
                    &active,
                    Duration::from_millis(10),
                    std::future::pending::<()>(),
                )
                .await,
                NetworkWait::TimedOut
            );

            let cancelling = StreamingState::new();
            let wait = wait_for_network(
                &cancelling,
                Duration::from_secs(1),
                std::future::pending::<()>(),
            );
            tokio::pin!(wait);
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut wait)
                    .await
                    .is_err(),
                "network wait must be registered and pending before cancel"
            );
            cancelling.cancel();
            assert_eq!(
                tokio::time::timeout(Duration::from_millis(50), wait)
                    .await
                    .expect("cancellation must wake network wait"),
                NetworkWait::Cancelled
            );
        });
    }

    #[test]
    fn production_streaming_has_no_per_track_client_construction() {
        let production = include_str!("streaming.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!production.contains("reqwest::Client::new()"));
        assert!(!production.contains("let client = reqwest::Client::new()"));
    }
}
