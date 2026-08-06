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
use tracing::debug;

use crate::decoder::mp3::{Mp3FrameDecoder, Mp3Step, STREAM_LOOKAHEAD_BYTES, append_as_f32};
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
    Samples(PcmChunk),
    /// Download/decode complete
    Done,
    /// Error occurred
    Error(String),
}

/// Samples a recycled PCM buffer starts out able to hold.
///
/// Roughly a third of a second of 48 kHz stereo, which covers the network chunk
/// sizes reqwest actually delivers. A larger chunk grows the buffer once and the
/// recycled buffer keeps the larger capacity, so growth is paid at most once per
/// stream rather than once per chunk.
const PCM_CHUNK_SAMPLES: usize = 32 * 1024;

/// Decoded PCM on loan from the stream that produced it.
///
/// A decoded chunk has to change threads, so it has to be owned; without a way
/// back, every chunk is one allocation on the streaming worker and one free on
/// the audio thread, forever, for as long as anything is playing. Handing the
/// buffer back is a `Drop` rather than a call the consumer has to remember,
/// which is the same instrument the render path's command vectors use: there is
/// no recycle step to forget.
pub struct PcmChunk {
    buffer: Vec<f32>,
    home: mpsc::Sender<Vec<f32>>,
}

impl PcmChunk {
    /// The buffer to decode into. Only the producer has one of these before it
    /// is sent, so this cannot be used to mutate a chunk in flight.
    pub(crate) fn buffer_mut(&mut self) -> &mut Vec<f32> {
        &mut self.buffer
    }
}

impl std::ops::Deref for PcmChunk {
    type Target = [f32];

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl Drop for PcmChunk {
    fn drop(&mut self) {
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.clear();
        // Never blocks and never grows. A full return channel means the producer
        // already holds more buffers than it can be using, and a closed one means
        // the stream is gone; either way this buffer is simply released. That
        // matters because the common caller is the audio thread.
        let _ = self.home.try_send(buffer);
    }
}

/// The producer's half of the loan: hands out chunks, takes back what returns.
pub(crate) struct PcmPool {
    home: mpsc::Sender<Vec<f32>>,
    returns: mpsc::Receiver<Vec<f32>>,
}

impl PcmPool {
    pub(crate) fn new() -> Self {
        // One slot more than can be in flight, so a healthy stream never fails a
        // return for want of room.
        let (home, returns) = mpsc::channel(STREAM_CHANNEL_CAPACITY + 1);
        Self { home, returns }
    }

    pub(crate) fn take(&mut self) -> PcmChunk {
        let buffer = self
            .returns
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(PCM_CHUNK_SAMPLES));
        PcmChunk {
            buffer,
            home: self.home.clone(),
        }
    }
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
    let mut pcm_pool = PcmPool::new();
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
        //
        // The buffer it decodes into is on loan from the pool: whatever the player
        // finished with is what this chunk is written into.
        let mut pcm = pcm_pool.take();
        let (rest, (pcm, sample_rate, channels)) = decoder
            .with(move |decoder| {
                decoder.push_data(&chunk);
                let (sample_rate, channels) = decoder.decode_into(pcm.buffer_mut());
                (pcm, sample_rate, channels)
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

        // Send decoded samples. A chunk that decoded to nothing is not sent; it
        // falls out of scope here, which is what returns its buffer to the pool.
        if !pcm.is_empty() {
            total_decoded_samples += pcm.len();
            tracing::trace!(
                "Decoded {} samples (total: {})",
                pcm.len(),
                total_decoded_samples
            );
            if tx.send(StreamMsg::Samples(pcm)).await.is_err() {
                return Ok(());
            }
        }
    }

    // Flush remaining data
    let mut final_pcm = pcm_pool.take();
    let (_decoder, (final_pcm, _, _)) = decoder
        .with(move |decoder| {
            let (sample_rate, channels) = decoder.flush_into(final_pcm.buffer_mut());
            (final_pcm, sample_rate, channels)
        })
        .await?;

    if !final_pcm.is_empty() {
        total_decoded_samples += final_pcm.len();
        tracing::trace!("Flushed {} final samples", final_pcm.len());
        if tx.send(StreamMsg::Samples(final_pcm)).await.is_err() {
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

/// Bytes of undecoded MP3 the receive buffer starts out able to hold.
///
/// Enough for a typical network chunk plus the lookahead the decoder keeps
/// behind its cursor, so the buffer settles at one allocation for the stream.
const RECEIVE_BUFFER_BYTES: usize = 64 * 1024;

/// MP3 streaming decoder.
///
/// Everything here is owned once and reused: the minimp3 decoder (whose state a
/// frame's main data is entitled to reach back into), the receive buffer, and
/// the scratch a frame is converted through on the way to the resampler. What a
/// chunk costs is a memcpy of its bytes and the samples it produces, and nothing
/// else.
struct Mp3StreamDecoder {
    frames: Mp3FrameDecoder,
    /// Bytes received and not yet decoded.
    buffer: Vec<u8>,
    /// Detected sample rate
    sample_rate: u32,
    /// Detected channels
    channels: u32,
    /// Target sample rate for resampling
    target_sample_rate: u32,
    /// Resampler state (if needed)
    resampler: Option<crate::resampler::StreamResampler>,
    /// One frame as `f32`, reused. Only the resampling path needs it; without a
    /// resampler a frame goes straight into the caller's buffer.
    scratch: Vec<f32>,
}

impl Mp3StreamDecoder {
    fn new(target_sample_rate: u32) -> Self {
        Self {
            frames: Mp3FrameDecoder::new(),
            buffer: Vec::with_capacity(RECEIVE_BUFFER_BYTES),
            sample_rate: 0,
            channels: 0,
            target_sample_rate,
            resampler: None,
            scratch: Vec::with_capacity(minimp3::MAX_SAMPLES_PER_FRAME),
        }
    }

    fn push_data(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Decode what the buffer can spare, appending to `out`.
    ///
    /// Leaves a lookahead behind the cursor so minimp3 can confirm the next
    /// frame's header and keep its state; see [`STREAM_LOOKAHEAD_BYTES`].
    ///
    /// **Known limitation, measured rather than assumed.** A stream whose whole
    /// body is shorter than the lookahead decodes nothing until the flush, and
    /// the flush then meets a decoder that has never run. minimp3's cold path
    /// will not accept a frame it cannot chain to a successor, so if such a
    /// stream also ends in bytes that are not audio — an ID3v1 tag is 128 bytes
    /// at the end of a large fraction of real files — the chain check fails at
    /// the tag and every frame is rejected. Under ~0.2 s of 128 kb/s audio, so
    /// only very short remote clips are exposed.
    ///
    /// Decoding eagerly until the first frame lands was tried and made things
    /// worse, which is why the constant is unconditional: it recovered that case
    /// only partly (1 frame of 6) while costing a frame on streams that were
    /// previously exact (23 of 24 instead of 24 of 24). Closing it properly means
    /// parsing frame lengths here rather than asking minimp3 for them, which is
    /// a bigger change than the case justifies.
    fn decode_into(&mut self, out: &mut Vec<f32>) -> (u32, u32) {
        self.decode_with_lookahead(STREAM_LOOKAHEAD_BYTES, out)
    }

    /// Decode everything that is left, for a stream that has ended.
    fn flush_into(&mut self, out: &mut Vec<f32>) -> (u32, u32) {
        self.decode_with_lookahead(0, out)
    }

    fn decode_with_lookahead(&mut self, lookahead: usize, out: &mut Vec<f32>) -> (u32, u32) {
        let mut pos = 0usize;

        while self.buffer.len().saturating_sub(pos) > lookahead {
            match self.frames.decode(&self.buffer[pos..]) {
                Mp3Step::NeedMoreData => break,
                Mp3Step::Skipped(skipped) => pos += skipped,
                Mp3Step::Frame {
                    pcm,
                    sample_rate,
                    channels,
                    consumed,
                } => {
                    pos += consumed;

                    if self.sample_rate == 0 {
                        self.sample_rate = sample_rate;
                        self.channels = channels;

                        if sample_rate != self.target_sample_rate {
                            self.resampler = Some(crate::resampler::StreamResampler::new(
                                sample_rate,
                                self.target_sample_rate,
                                channels,
                            ));
                        }
                    }

                    match self.resampler.as_mut() {
                        Some(resampler) => {
                            self.scratch.clear();
                            append_as_f32(pcm, &mut self.scratch);
                            resampler.process_into(&self.scratch, out);
                        }
                        None => append_as_f32(pcm, out),
                    }
                }
            }
        }

        if pos > 0 {
            self.buffer.drain(..pos);
        }

        (self.sample_rate, self.channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp3_fixture;
    use migo_alloc_probe::{Burst, assert_no_steady_state_allocation};
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
                        decoder.decode_into(&mut Vec::new());
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

    /// Section 7.3's steady-state allocation gate, applied to the streaming
    /// refill: one network chunk in, its decoded PCM out.
    ///
    /// The unit is a chunk because that is what repeats -- a track is thousands
    /// of them -- and everything inside it is per-frame, so an allocation here is
    /// multiplied by however many frames the chunk carried. Resampling is on
    /// (the fixture is 44.1 kHz, the device 48 kHz), which is the configuration
    /// most Android devices actually run.
    ///
    /// The warm-up covers the decoder's first frame, the resampler's
    /// construction, every buffer's first growth to a chunk's worth, and the
    /// return channel's block list reaching its working set -- tokio hands out
    /// message slots 32 to a block and recycles them, so the list grows once and
    /// then never again. Forty iterations crosses that boundary; the property was
    /// checked over 256 measured iterations before this window was chosen, so the
    /// number is a warm-up and not a hiding place.
    #[test]
    fn a_steady_state_streaming_chunk_never_reaches_the_heap() {
        const WARMUP: usize = 40;
        const MEASURED: usize = 128;
        const FRAMES_PER_CHUNK: usize = 2;

        let chunk_bytes = FRAMES_PER_CHUNK * mp3_fixture::FRAME_BYTES;
        // Built before the measured window, and long enough that no iteration
        // runs out of input and quietly measures a decoder with nothing to do.
        let source = mp3_fixture::stream(FRAMES_PER_CHUNK * (WARMUP + MEASURED) + 4);
        let chunks: Vec<&[u8]> = source.chunks_exact(chunk_bytes).collect();
        assert!(chunks.len() >= WARMUP + MEASURED);

        let mut decoder = Mp3StreamDecoder::new(48_000);
        let mut pool = PcmPool::new();
        let mut decoded_total = 0usize;

        assert_no_steady_state_allocation(
            Burst {
                path: "audio: one streaming chunk decoded and resampled",
                warmup: WARMUP,
                measured: MEASURED,
            },
            |iteration| {
                decoder.push_data(chunks[iteration]);
                // Taken from the pool and dropped again at the end of the
                // iteration, which is the whole loan: a burst that kept its
                // buffers would measure the pool draining, not the steady state.
                let mut pcm = pool.take();
                decoder.decode_into(pcm.buffer_mut());
                decoded_total += pcm.len();
            },
        );

        assert!(
            decoded_total > 0,
            "the burst must have decoded audio, or it proves nothing"
        );
    }

    /// A stream cut into chunks must decode to exactly what one pass over the
    /// same bytes produces.
    ///
    /// This is the correctness half of the same change, and it is a reftest
    /// rather than a golden file because both sides are computed in the same run:
    /// no baseline to go stale, and no claim about any particular recording. The
    /// chunk size is deliberately not a multiple of the frame size, so frames are
    /// split at every offset within them.
    ///
    /// What it pins is decoder *state*. Rebuilding the decoder per chunk -- which
    /// is what constructing one inside the decode step amounted to -- discards
    /// the bit reservoir, and a frame whose main data lives there then decodes to
    /// nothing at all: silently short audio, no error anywhere.
    #[test]
    fn a_chunked_stream_decodes_to_what_one_pass_over_the_same_bytes_does() {
        const FRAMES: usize = 24;
        // Coprime with the frame size, so no chunk boundary lands on a frame one.
        const CHUNK_BYTES: usize = 137;

        let source = mp3_fixture::stream(FRAMES);
        let one_pass = crate::decoder::mp3::decode(&source).expect("the fixture must decode");

        // Same rate in and out, so the two sides are comparable sample for sample.
        let mut decoder = Mp3StreamDecoder::new(mp3_fixture::SAMPLE_RATE);
        let mut chunked = Vec::new();
        for chunk in source.chunks(CHUNK_BYTES) {
            decoder.push_data(chunk);
            decoder.decode_into(&mut chunked);
        }
        decoder.flush_into(&mut chunked);

        assert_eq!(
            one_pass.samples.len(),
            FRAMES * mp3_fixture::SAMPLES_PER_FRAME,
            "the one-pass side must itself be complete, or this compares two \
             equally broken decodes"
        );
        assert_eq!(
            chunked.len(),
            one_pass.samples.len(),
            "a chunked stream lost or invented samples"
        );
        assert_eq!(chunked, one_pass.samples);
    }

    /// A stream that ends in bytes that are not audio must still decode.
    ///
    /// An ID3v1 tag is 128 bytes at the very end of a large fraction of real MP3
    /// files, and minimp3 will not accept a frame it cannot chain to a successor,
    /// so the frame immediately before such a tag is lost. That single frame —
    /// 26 ms at the end of a track — is the accepted cost; what is asserted here
    /// is that it is *one* frame and not the stream.
    ///
    /// **Only streams longer than the lookahead are asserted here, and that is a
    /// stated limitation rather than a convenient choice of input.** A stream
    /// whose whole body fits inside the lookahead decodes nothing until the
    /// flush and meets a cold decoder there, which rejects the whole chain; see
    /// `Mp3StreamDecoder::decode_into`. Measured at 0 frames of 6, against 2 of 6
    /// for the per-chunk decoder this replaced — worse in that one cell, and
    /// better everywhere else, which is why it is written down here instead of
    /// being covered by picking a longer fixture and saying nothing.
    #[test]
    fn a_stream_ending_in_a_tag_loses_at_most_its_final_frame() {
        let id3v1 = {
            let mut tag = b"TAG".to_vec();
            tag.resize(128, 0);
            tag
        };

        for frames in [40usize, 96] {
            let mut source = mp3_fixture::stream(frames);
            source.extend_from_slice(&id3v1);

            let mut decoder = Mp3StreamDecoder::new(mp3_fixture::SAMPLE_RATE);
            let mut out = Vec::new();
            for chunk in source.chunks(137) {
                decoder.push_data(chunk);
                decoder.decode_into(&mut out);
            }
            decoder.flush_into(&mut out);

            let decoded = out.len() / mp3_fixture::SAMPLES_PER_FRAME;
            assert!(
                decoded >= frames - 1,
                "a {frames}-frame stream ending in an ID3v1 tag decoded {decoded} frames; \
                 at most the final frame may be lost to the tag"
            );
        }
    }

    /// The loan has to come back, and come back empty.
    ///
    /// Identity is asserted by address: a pool that quietly allocated a fresh
    /// buffer each time would satisfy every other observable property here, and
    /// the allocation gate above is the only other thing that would notice.
    #[test]
    fn a_returned_chunk_is_the_buffer_the_next_one_is_written_into() {
        let mut pool = PcmPool::new();

        let mut first = pool.take();
        first.buffer_mut().extend_from_slice(&[0.5f32; 128]);
        let lent = first.buffer.as_ptr();
        assert_eq!(first.len(), 128);
        drop(first);

        let second = pool.take();
        assert!(second.is_empty(), "a returned buffer must come back empty");
        assert_eq!(
            second.buffer.as_ptr(),
            lent,
            "the next chunk must be written into the buffer the last one returned"
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
