//! InnerAudioContext implementation - simple audio playback without WebAudio graph complexity.
//!
//! This is designed for simple media playback (background music, sound effects)
//! where the full WebAudio API is not needed.
//!
//! Supports streaming playback - can start playing before full download completes.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use shared::protocol::audio_cmd::{InnerAudioEvent, InnerAudioEventType, InnerAudioId};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::decoder::DecodedAudio;
use crate::streaming::{StreamMsg, StreamingState};

/// Playback state for InnerAudioContext
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PlaybackState {
    /// Initial state, no audio loaded or stopped
    Stopped = 0,
    /// Audio is playing
    Playing = 1,
    /// Audio is paused
    Paused = 2,
}

impl From<u8> for PlaybackState {
    fn from(v: u8) -> Self {
        match v {
            1 => PlaybackState::Playing,
            2 => PlaybackState::Paused,
            _ => PlaybackState::Stopped,
        }
    }
}

/// Shared state between InnerAudioPlayer and the mixer
/// All fields are atomic for lock-free access from audio callback
pub struct InnerAudioSharedState {
    /// Current playback state
    state: AtomicU32,
    /// Current playback position in sample frames (atomic u64 for precision)
    position_frames: AtomicU64,
    /// Total duration in sample frames
    duration_frames: AtomicU64,
    /// Volume (0.0 - 1.0, stored as u32 with fixed point: value * 1000)
    volume: AtomicU32,
    /// Playback rate (stored as u32 with fixed point: rate * 1000)
    playback_rate: AtomicU32,
    /// Loop enabled
    loop_enabled: AtomicBool,
    /// Seek target in frames (u64::MAX means no seek pending)
    seek_target: AtomicU64,
    /// Sample rate of the audio
    sample_rate: AtomicU32,
    /// Number of channels
    channels: AtomicU32,
    /// Autoplay flag
    autoplay: AtomicBool,
    /// Whether audio data is loaded
    loaded: AtomicBool,
}

impl InnerAudioSharedState {
    pub fn new() -> Self {
        Self {
            state: AtomicU32::new(PlaybackState::Stopped as u32),
            position_frames: AtomicU64::new(0),
            duration_frames: AtomicU64::new(0),
            volume: AtomicU32::new(1000),        // 1.0
            playback_rate: AtomicU32::new(1000), // 1.0
            loop_enabled: AtomicBool::new(false),
            seek_target: AtomicU64::new(u64::MAX),
            sample_rate: AtomicU32::new(0),
            channels: AtomicU32::new(0),
            autoplay: AtomicBool::new(false),
            loaded: AtomicBool::new(false),
        }
    }

    #[inline]
    pub fn state(&self) -> PlaybackState {
        PlaybackState::from(self.state.load(Ordering::Acquire) as u8)
    }

    #[inline]
    pub fn set_state(&self, state: PlaybackState) {
        self.state.store(state as u32, Ordering::Release);
    }

    #[inline]
    pub fn position_frames(&self) -> u64 {
        self.position_frames.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_position_frames(&self, frames: u64) {
        self.position_frames.store(frames, Ordering::Relaxed);
    }

    #[inline]
    pub fn duration_frames(&self) -> u64 {
        self.duration_frames.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_duration_frames(&self, frames: u64) {
        self.duration_frames.store(frames, Ordering::Relaxed);
    }

    /// Get volume as f32 (0.0 - 1.0)
    #[inline]
    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Set volume (0.0 - 1.0)
    #[inline]
    pub fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 1.0);
        self.volume
            .store((clamped * 1000.0) as u32, Ordering::Relaxed);
    }

    /// Get playback rate as f32
    #[inline]
    pub fn playback_rate(&self) -> f32 {
        self.playback_rate.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Set playback rate (0.5 - 2.0)
    #[inline]
    pub fn set_playback_rate(&self, rate: f32) {
        let clamped = rate.clamp(0.5, 2.0);
        self.playback_rate
            .store((clamped * 1000.0) as u32, Ordering::Relaxed);
    }

    #[inline]
    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_loop_enabled(&self, enabled: bool) {
        self.loop_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Request a seek to the given frame position
    #[inline]
    pub fn request_seek(&self, frame: u64) {
        self.seek_target.store(frame, Ordering::Release);
    }

    /// Check and clear seek target. Returns Some(frame) if seek was requested.
    #[inline]
    pub fn take_seek_target(&self) -> Option<u64> {
        let target = self.seek_target.swap(u64::MAX, Ordering::AcqRel);
        if target == u64::MAX {
            None
        } else {
            Some(target)
        }
    }

    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_sample_rate(&self, rate: u32) {
        self.sample_rate.store(rate, Ordering::Relaxed);
    }

    #[inline]
    pub fn channels(&self) -> u32 {
        self.channels.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_channels(&self, ch: u32) {
        self.channels.store(ch, Ordering::Relaxed);
    }

    #[inline]
    pub fn autoplay(&self) -> bool {
        self.autoplay.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_autoplay(&self, enabled: bool) {
        self.autoplay.store(enabled, Ordering::Relaxed);
    }

    #[inline]
    pub fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set_loaded(&self, loaded: bool) {
        self.loaded.store(loaded, Ordering::Release);
    }

    /// Get current time in seconds
    pub fn current_time(&self) -> f64 {
        let sr = self.sample_rate();
        if sr == 0 {
            return 0.0;
        }
        self.position_frames() as f64 / sr as f64
    }

    /// Get duration in seconds
    pub fn duration(&self) -> f64 {
        let sr = self.sample_rate();
        if sr == 0 {
            return 0.0;
        }
        self.duration_frames() as f64 / sr as f64
    }

    /// Seek to time in seconds
    pub fn seek(&self, time: f64) {
        let sr = self.sample_rate();
        if sr == 0 {
            return;
        }
        let frame = (time * sr as f64).max(0.0) as u64;
        let duration = self.duration_frames();
        let clamped = frame.min(duration);
        self.request_seek(clamped);
    }
}

impl Default for InnerAudioSharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio data source - either owned (streaming) or shared (cached)
enum AudioSource {
    /// Owned samples (used during streaming, can grow)
    Owned(Vec<f32>),
    /// Shared cached samples (immutable, from cache)
    Cached(Arc<DecodedAudio>),
}

impl AudioSource {
    fn samples(&self) -> &[f32] {
        match self {
            AudioSource::Owned(v) => v,
            AudioSource::Cached(a) => &a.samples,
        }
    }

    fn len(&self) -> usize {
        match self {
            AudioSource::Owned(v) => v.len(),
            AudioSource::Cached(a) => a.samples.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Extend owned samples (only works for Owned variant)
    fn extend(&mut self, samples: Vec<f32>) {
        if let AudioSource::Owned(v) = self {
            v.extend(samples);
        }
    }
}

/// InnerAudioPlayer - handles playback of a single audio source
///
/// This is stored in the audio thread and processes samples for output.
/// Supports both full-load and streaming playback modes.
pub struct InnerAudioPlayer {
    /// Unique ID for this player
    pub id: InnerAudioId,
    /// Shared state for communication with JS
    pub shared: Arc<InnerAudioSharedState>,
    /// Audio data source (owned or cached)
    source: AudioSource,
    /// Current position in samples (not frames)
    position: usize,
    /// Output channels from device
    output_channels: u32,
    /// Pending events to send back to JS
    pending_events: Vec<InnerAudioEvent>,
    /// Streaming receiver (if streaming mode)
    stream_rx: Option<UnboundedReceiver<StreamMsg>>,
    /// Streaming state for progress tracking
    stream_state: Option<Arc<StreamingState>>,
    /// Whether streaming download is complete
    stream_complete: bool,
    /// Whether we've sent canplay event for streaming
    stream_canplay_sent: bool,
    /// URL being loaded (for cache key)
    loading_url: Option<String>,
}

impl InnerAudioPlayer {
    pub fn new(id: InnerAudioId, output_channels: u32) -> Self {
        Self {
            id,
            shared: Arc::new(InnerAudioSharedState::new()),
            source: AudioSource::Owned(Vec::new()),
            position: 0,
            output_channels,
            pending_events: Vec::new(),
            stream_rx: None,
            stream_state: None,
            stream_complete: false,
            stream_canplay_sent: false,
            loading_url: None,
        }
    }

    /// Helper to push an event with current time
    #[inline]
    fn push_event(&mut self, event_type: InnerAudioEventType) {
        self.pending_events.push(InnerAudioEvent {
            id: self.id,
            event_type,
            current_time: self.shared.current_time(),
        });
    }

    /// Load decoded audio data (full load mode, owned)
    pub fn load_audio(&mut self, audio: DecodedAudio) {
        // Cancel any ongoing stream
        self.cancel_stream();

        let frame_count = audio.frame_count();
        tracing::debug!(
            "InnerAudioPlayer {} load_audio: {} frames, {} Hz, {} ch, {:.2}s",
            self.id,
            frame_count,
            audio.sample_rate,
            audio.channels,
            frame_count as f64 / audio.sample_rate as f64
        );

        self.shared.set_sample_rate(audio.sample_rate);
        self.shared.set_channels(audio.channels);
        self.shared.set_duration_frames(frame_count as u64);
        self.shared.set_position_frames(0);
        self.source = AudioSource::Owned(audio.samples);
        self.position = 0;
        self.stream_complete = true;
        self.loading_url = None;
        self.shared.set_loaded(true);
        self.push_event(InnerAudioEventType::CanPlay);

        // Auto-play if requested
        if self.shared.autoplay() {
            tracing::debug!("InnerAudioPlayer {} autoplay triggered", self.id);
            self.shared.set_state(PlaybackState::Playing);
            self.push_event(InnerAudioEventType::Play);
        }
    }

    /// Load cached audio data (shared reference, no copy)
    pub fn load_cached(&mut self, audio: Arc<DecodedAudio>) {
        // Cancel any ongoing stream
        self.cancel_stream();

        let frame_count = audio.frame_count();
        tracing::debug!(
            "InnerAudioPlayer {} load_cached: {} frames, {} Hz, {} ch, {:.2}s",
            self.id,
            frame_count,
            audio.sample_rate,
            audio.channels,
            frame_count as f64 / audio.sample_rate as f64
        );

        self.shared.set_sample_rate(audio.sample_rate);
        self.shared.set_channels(audio.channels);
        self.shared.set_duration_frames(frame_count as u64);
        self.shared.set_position_frames(0);
        self.source = AudioSource::Cached(audio);
        self.position = 0;
        self.stream_complete = true;
        self.loading_url = None;
        self.shared.set_loaded(true);
        self.push_event(InnerAudioEventType::CanPlay);

        // Auto-play if requested
        if self.shared.autoplay() {
            tracing::debug!("InnerAudioPlayer {} autoplay triggered", self.id);
            self.shared.set_state(PlaybackState::Playing);
            self.push_event(InnerAudioEventType::Play);
        }
    }

    /// Start streaming playback from URL
    pub fn start_streaming(
        &mut self,
        url: String,
        rx: UnboundedReceiver<StreamMsg>,
        state: Arc<StreamingState>,
    ) {
        // Cancel any previous stream
        self.cancel_stream();

        // Reset state with modest pre-allocated buffer
        // ~5 seconds stereo at 44.1kHz = 44100 * 2 * 5 = 441000 samples (~1.7MB)
        const ESTIMATED_CAPACITY: usize = 44100 * 2 * 5;
        self.source = AudioSource::Owned(Vec::with_capacity(ESTIMATED_CAPACITY));
        self.position = 0;
        self.shared.set_position_frames(0);
        self.shared.set_duration_frames(0);
        self.shared.set_sample_rate(0);
        self.shared.set_channels(0);
        self.shared.set_loaded(false);
        self.stream_complete = false;
        self.stream_canplay_sent = false;
        self.loading_url = Some(url);

        // Set streaming receiver
        self.stream_rx = Some(rx);
        self.stream_state = Some(state);

        tracing::debug!("InnerAudioPlayer {} started streaming", self.id);
    }

    /// Get the URL currently being loaded (for cache key)
    pub fn loading_url(&self) -> Option<&str> {
        self.loading_url.as_deref()
    }

    /// Take ownership of streamed samples for caching
    /// Only call after stream is complete
    pub fn take_streamed_audio(&mut self) -> Option<DecodedAudio> {
        if !self.stream_complete {
            return None;
        }

        let samples = match std::mem::replace(&mut self.source, AudioSource::Owned(Vec::new())) {
            AudioSource::Owned(samples) => samples,
            AudioSource::Cached(_) => return None, // Already cached
        };

        if samples.is_empty() {
            return None;
        }

        Some(DecodedAudio {
            samples,
            sample_rate: self.shared.sample_rate(),
            channels: self.shared.channels(),
        })
    }

    /// Cancel ongoing stream
    fn cancel_stream(&mut self) {
        if let Some(state) = self.stream_state.take() {
            state.cancel();
        }
        self.stream_rx = None;
    }

    /// Process incoming stream messages (call from audio thread loop)
    pub fn poll_stream(&mut self) {
        // First, collect all messages to avoid borrow conflicts
        let messages: Vec<StreamMsg> = {
            let rx = match &mut self.stream_rx {
                Some(rx) => rx,
                None => return,
            };

            let mut msgs = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                msgs.push(msg);
            }
            msgs
        };

        // Now process collected messages
        for msg in messages {
            match msg {
                StreamMsg::Ready {
                    sample_rate,
                    channels,
                } => {
                    self.shared.set_sample_rate(sample_rate);
                    self.shared.set_channels(channels);
                    tracing::debug!(
                        "InnerAudioPlayer {} stream ready: {} Hz, {} channels",
                        self.id,
                        sample_rate,
                        channels
                    );
                }
                StreamMsg::Samples(new_samples) => {
                    self.source.extend(new_samples);

                    // Update duration as we receive more data
                    let channels = self.shared.channels() as usize;
                    if channels > 0 {
                        let frames = self.source.len() / channels;
                        self.shared.set_duration_frames(frames as u64);
                    }

                    // Send canplay once we have enough data (~0.5 seconds)
                    if !self.stream_canplay_sent {
                        let sr = self.shared.sample_rate();
                        let ch = self.shared.channels();
                        if sr > 0 && ch > 0 {
                            let buffered_frames = self.source.len() / ch as usize;
                            let buffered_seconds = buffered_frames as f64 / sr as f64;

                            tracing::trace!(
                                "InnerAudioPlayer {} buffered: {:.2}s ({} frames, {} samples)",
                                self.id,
                                buffered_seconds,
                                buffered_frames,
                                self.source.len()
                            );

                            // Can play once we have 0.5 seconds buffered
                            if buffered_seconds >= 0.5 {
                                self.stream_canplay_sent = true;
                                self.shared.set_loaded(true);
                                tracing::debug!(
                                    "InnerAudioPlayer {} canplay: {:.2}s buffered",
                                    self.id,
                                    buffered_seconds
                                );
                                self.push_event(InnerAudioEventType::CanPlay);

                                // Auto-play if requested
                                if self.shared.autoplay() {
                                    tracing::debug!(
                                        "InnerAudioPlayer {} autoplay triggered",
                                        self.id
                                    );
                                    self.shared.set_state(PlaybackState::Playing);
                                    self.push_event(InnerAudioEventType::Play);
                                }
                            }
                        }
                    }
                }
                StreamMsg::Done => {
                    self.stream_complete = true;
                    self.stream_rx = None;
                    self.stream_state = None;
                    let ch = self.shared.channels() as usize;
                    let sr = self.shared.sample_rate();
                    let duration = if ch > 0 && sr > 0 {
                        (self.source.len() / ch) as f64 / sr as f64
                    } else {
                        0.0
                    };
                    tracing::debug!(
                        "InnerAudioPlayer {} stream complete: {} samples, {:.2}s duration",
                        self.id,
                        self.source.len(),
                        duration
                    );

                    // If we haven't sent canplay yet (very short audio), send it now
                    if !self.stream_canplay_sent && !self.source.is_empty() {
                        self.stream_canplay_sent = true;
                        self.shared.set_loaded(true);
                        tracing::debug!("InnerAudioPlayer {} canplay (short audio)", self.id);
                        self.push_event(InnerAudioEventType::CanPlay);

                        if self.shared.autoplay() {
                            tracing::debug!("InnerAudioPlayer {} autoplay triggered", self.id);
                            self.shared.set_state(PlaybackState::Playing);
                            self.push_event(InnerAudioEventType::Play);
                        }
                    }
                }
                StreamMsg::Error(err) => {
                    tracing::error!("InnerAudioPlayer {} stream error: {}", self.id, err);
                    self.stream_rx = None;
                    self.stream_state = None;
                    self.push_event(InnerAudioEventType::Error);
                }
            }
        }
    }

    /// Play the audio
    pub fn play(&mut self) {
        if !self.shared.is_loaded() {
            tracing::trace!("InnerAudioPlayer {} play() ignored: not loaded", self.id);
            return;
        }
        let prev_state = self.shared.state();
        if prev_state != PlaybackState::Playing {
            tracing::debug!(
                "InnerAudioPlayer {} play: {:?} -> Playing",
                self.id,
                prev_state
            );
            self.shared.set_state(PlaybackState::Playing);
            self.push_event(InnerAudioEventType::Play);
        }
    }

    /// Pause the audio
    pub fn pause(&mut self) {
        if self.shared.state() == PlaybackState::Playing {
            tracing::debug!(
                "InnerAudioPlayer {} pause at {:.2}s",
                self.id,
                self.shared.current_time()
            );
            self.shared.set_state(PlaybackState::Paused);
            self.push_event(InnerAudioEventType::Pause);
        }
    }

    /// Stop the audio and reset position
    pub fn stop(&mut self) {
        tracing::debug!("InnerAudioPlayer {} stop", self.id);
        self.shared.set_state(PlaybackState::Stopped);
        self.position = 0;
        self.shared.set_position_frames(0);
        self.push_event(InnerAudioEventType::Stop);
    }

    /// Take pending events (call from audio thread loop)
    pub fn take_events(&mut self) -> Vec<InnerAudioEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Check if player is active (playing)
    #[inline]
    pub fn is_active(&self) -> bool {
        self.shared.state() == PlaybackState::Playing && self.shared.is_loaded()
    }

    /// Process and mix samples into output buffer
    /// Returns true if still playing, false if finished
    pub fn process(&mut self, output: &mut [f32]) -> bool {
        if !self.is_active() {
            return self.shared.state() != PlaybackState::Stopped;
        }

        let channels = self.shared.channels() as usize;
        if channels == 0 || self.source.is_empty() {
            return false;
        }

        // Handle pending seek
        if let Some(target_frame) = self.shared.take_seek_target() {
            self.push_event(InnerAudioEventType::Seeking);
            self.position = (target_frame as usize) * channels;
            self.position = self.position.min(self.source.len());
            self.shared
                .set_position_frames(self.position as u64 / channels as u64);
            self.push_event(InnerAudioEventType::Seeked);
        }

        let volume = self.shared.volume();
        let playback_rate = self.shared.playback_rate();
        let loop_enabled = self.shared.loop_enabled();
        let output_channels = self.output_channels as usize;

        // Get samples reference after seek handling
        let samples = self.source.samples();

        // Simple playback rate handling (no interpolation for now, just skip/repeat frames)
        // We track position in FRAMES (not samples) using 16.16 fixed point
        let rate_fixed = (playback_rate * 65536.0) as u64;
        let current_frame = self.position / channels;
        let mut frame_fixed = (current_frame as u64) << 16; // 16.16 fixed point for frame position

        let total_frames = samples.len() / channels;
        let output_frames = output.len() / output_channels;

        for frame_idx in 0..output_frames {
            let src_frame = (frame_fixed >> 16) as usize;
            let src_sample_start = src_frame * channels;

            // Check if we've reached the end of available data
            if src_frame >= total_frames {
                // In streaming mode, we might just need to wait for more data
                if !self.stream_complete && self.stream_rx.is_some() {
                    // Streaming but caught up - output silence for remaining
                    tracing::trace!(
                        "InnerAudioPlayer {} waiting for stream data (frame {} >= available {})",
                        self.id,
                        src_frame,
                        total_frames
                    );
                    let start = frame_idx * output_channels;
                    for s in output[start..].iter_mut() {
                        *s = 0.0;
                    }
                    // Keep position at end of available data
                    let final_frame = total_frames.saturating_sub(1);
                    self.position = final_frame * channels;
                    self.shared.set_position_frames(final_frame as u64);
                    // TODO: could emit Waiting event here
                    return true; // Still playing, waiting for data
                }

                if loop_enabled {
                    // Loop back to start
                    tracing::trace!("InnerAudioPlayer {} looping back to start", self.id);
                    frame_fixed = 0;
                    continue;
                } else {
                    // Playback finished
                    tracing::debug!("InnerAudioPlayer {} playback ended", self.id);
                    self.shared.set_state(PlaybackState::Stopped);
                    self.push_event(InnerAudioEventType::Ended);

                    // Fill remaining with silence
                    let start = frame_idx * output_channels;
                    for s in output[start..].iter_mut() {
                        *s = 0.0;
                    }
                    self.position = 0;
                    self.shared.set_position_frames(0);
                    return false;
                }
            }

            // Mix into output (handle channel conversion)
            let dst_start = frame_idx * output_channels;
            let total_samples = samples.len();

            if channels == output_channels {
                // Same channel count - direct mix
                for ch in 0..channels {
                    if src_sample_start + ch < total_samples && dst_start + ch < output.len() {
                        output[dst_start + ch] += samples[src_sample_start + ch] * volume;
                    }
                }
            } else if channels == 1 && output_channels == 2 {
                // Mono to stereo
                if src_sample_start < total_samples && dst_start + 1 < output.len() {
                    let sample = samples[src_sample_start] * volume;
                    output[dst_start] += sample;
                    output[dst_start + 1] += sample;
                }
            } else if channels == 2 && output_channels == 1 {
                // Stereo to mono
                if src_sample_start + 1 < total_samples && dst_start < output.len() {
                    let sample =
                        (samples[src_sample_start] + samples[src_sample_start + 1]) * 0.5 * volume;
                    output[dst_start] += sample;
                }
            } else {
                // Generic downmix/upmix - just use first channels
                let copy_channels = channels.min(output_channels);
                for ch in 0..copy_channels {
                    if src_sample_start + ch < total_samples && dst_start + ch < output.len() {
                        output[dst_start + ch] += samples[src_sample_start + ch] * volume;
                    }
                }
            }

            // Advance frame position by playback rate
            frame_fixed += rate_fixed;
        }

        // Update position (convert frame back to sample index)
        let final_frame = ((frame_fixed >> 16) as usize).min(total_frames);
        self.position = final_frame * channels;
        self.shared.set_position_frames(final_frame as u64);

        true
    }

    /// Check if stream download is complete (for caching)
    pub fn is_stream_complete(&self) -> bool {
        self.stream_complete
    }
}
