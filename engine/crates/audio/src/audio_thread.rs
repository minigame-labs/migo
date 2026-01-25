use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::audio_cmd::{
    AudioBufferInfo, AudioCmd, AudioContextId, AudioContextState,
    InnerAudioId, InnerAudioInfo, InnerAudioState,
};
use shared::protocol::host_cmd::{HostCommand, InnerAudioEventType};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{error, info};

use crate::cache::GlobalAudioCache;
use crate::context::AudioContext;
use crate::decoder;
use crate::inner_audio::{InnerAudioPlayer, PlaybackState};
use crate::output::AudioOutput;
use crate::resampler;
use crate::streaming::{self, StreamingState};

/// Result of thread initialization
enum InitResult {
    Ok(thread::ThreadId),
    Err(String),
}

pub struct AudioThread {
    tx: UnboundedSender<AudioCmd>,
    handle: Option<thread::JoinHandle<()>>,
    thread_id: thread::ThreadId,
}

impl AudioThread {
    pub fn spawn(host_tx: tokio::sync::mpsc::Sender<HostCommand>) -> EngineResult<Self> {
        let (tx, rx) = unbounded_channel::<AudioCmd>();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<InitResult>(1);

        let handle = thread::Builder::new()
            .name("MiniGame-AudioThread".into())
            .spawn(move || {
                // Initialize audio output
                let output = match AudioOutput::new() {
                    Ok(out) => {
                        let _ = init_tx.send(InitResult::Ok(thread::current().id()));
                        out
                    }
                    Err(e) => {
                        error!("Failed to initialize audio output: {}", e);
                        let _ = init_tx.send(InitResult::Err(e.to_string()));
                        return;
                    }
                };

                info!("AudioThread started");

                // Run the audio thread loop
                run_audio_thread(rx, output, host_tx);

                info!("AudioThread stopped");
            })
            .map_err(|e| {
                EngineError::from_detail(
                    ErrorCode::IoError,
                    format!("Failed to spawn audio thread: {}", e),
                )
            })?;

        // Wait for initialization result
        match init_rx.recv() {
            Ok(InitResult::Ok(thread_id)) => Ok(Self {
                tx,
                handle: Some(handle),
                thread_id,
            }),
            Ok(InitResult::Err(e)) => Err(EngineError::from_detail(
                ErrorCode::Internal,
                format!("Audio thread initialization failed: {}", e),
            )),
            Err(_) => Err(EngineError::from_detail(
                ErrorCode::Internal,
                "Audio thread terminated before initialization",
            )),
        }
    }

    #[inline]
    pub fn sender(&self) -> UnboundedSender<AudioCmd> {
        self.tx.clone()
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(AudioCmd::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        let _ = self.tx.send(AudioCmd::Shutdown);

        // Never join from inside the audio thread itself
        if thread::current().id() == self.thread_id {
            self.handle.take();
            return;
        }

        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Audio thread main loop
fn run_audio_thread(
    mut rx: UnboundedReceiver<AudioCmd>,
    mut output: AudioOutput,
    host_tx: tokio::sync::mpsc::Sender<HostCommand>,
) {
    let sample_rate = output.sample_rate();
    let channels = output.channels();

    let mut contexts: HashMap<AudioContextId, AudioContext> = HashMap::new();
    let mut next_context_id: AudioContextId = 1;

    // InnerAudioContext players
    let mut inner_players: HashMap<InnerAudioId, InnerAudioPlayer> = HashMap::new();

    // Global audio cache (64MB default)
    let audio_cache = GlobalAudioCache::new();

    // Audio processing buffer - process enough to fill buffer when needed
    // ~21ms at 48kHz stereo = 1024 frames
    const PROCESS_FRAMES: usize = 1024;
    let buffer_size = PROCESS_FRAMES * channels as usize;
    let mut process_buffer = vec![0.0f32; buffer_size];

    // Get sync handle for callback-driven wakeup
    let sync = output.sync().clone();

    loop {
        // Process commands (non-blocking)
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                AudioCmd::Shutdown => {
                    return;
                }

                AudioCmd::CreateContext {
                    sample_rate: req_rate,
                    resp,
                } => {
                    let rate = req_rate.unwrap_or(sample_rate);
                    let id = next_context_id;
                    next_context_id += 1;
                    contexts.insert(id, AudioContext::new(id, rate, channels));
                    let _ = resp.send(Ok(id));
                }

                AudioCmd::CloseContext { ctx_id, resp } => {
                    if let Some(mut ctx) = contexts.remove(&ctx_id) {
                        ctx.close();
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::GetContextState { ctx_id, resp } => {
                    if let Some(ctx) = contexts.get(&ctx_id) {
                        let _ = resp.send(Ok(ctx.state()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::ResumeContext { ctx_id, resp } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.resume();
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::SuspendContext { ctx_id, resp } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.suspend();
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::DecodeAudioData { ctx_id, data, resp } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        match decoder::decode(&data) {
                            Ok(decoded) => {
                                // Resample to output device sample rate if needed
                                match resampler::resample_if_needed(decoded, sample_rate) {
                                    Ok(resampled) => {
                                        let duration = resampled.duration();
                                        let sr = resampled.sample_rate;
                                        let ch = resampled.channels;
                                        let length = resampled.frame_count() as u32;
                                        let id = ctx.add_buffer(resampled);
                                        let _ = resp.send(Ok(AudioBufferInfo {
                                            id,
                                            duration,
                                            sample_rate: sr,
                                            channels: ch,
                                            length,
                                        }));
                                    }
                                    Err(e) => {
                                        let _ = resp.send(Err(e));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = resp.send(Err(e));
                            }
                        }
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::ReleaseBuffer { buffer_id, resp } => {
                    // Find and remove buffer from any context
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.remove_buffer(buffer_id) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioBuffer {} not found", buffer_id),
                        )));
                    }
                }

                AudioCmd::CreateBufferSource { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.create_buffer_source(node_id);
                    } else {
                        tracing::warn!("CreateBufferSource: AudioContext {} not found", ctx_id);
                    }
                }

                AudioCmd::SetBuffer {
                    node_id,
                    buffer_id,
                    resp,
                } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.set_buffer(node_id, buffer_id) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            "Node or buffer not found",
                        )));
                    }
                }

                AudioCmd::Start {
                    node_id,
                    when,
                    offset,
                    duration,
                    resp,
                } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.start_source(node_id, when, offset, duration) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioBufferSourceNode {} not found", node_id),
                        )));
                    }
                }

                AudioCmd::Stop { node_id, when, resp } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.stop_source(node_id, when) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioBufferSourceNode {} not found", node_id),
                        )));
                    }
                }

                AudioCmd::SetLoop {
                    node_id,
                    loop_enabled,
                    loop_start,
                    loop_end,
                } => {
                    tracing::trace!("SetLoop: node_id={}, enabled={}, start={}, end={}",
                        node_id, loop_enabled, loop_start, loop_end);
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.set_loop(node_id, loop_enabled, loop_start, loop_end) {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        tracing::warn!("SetLoop: node {} not found", node_id);
                    }
                }

                AudioCmd::SetPlaybackRate { node_id, rate, resp } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.set_playback_rate(node_id, rate) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioBufferSourceNode {} not found", node_id),
                        )));
                    }
                }

                AudioCmd::CreateGain { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.create_gain(node_id);
                    } else {
                        tracing::warn!("CreateGain: AudioContext {} not found", ctx_id);
                    }
                }

                AudioCmd::SetGainValue { node_id, value } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        if ctx.set_gain(node_id, value) {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        tracing::warn!("SetGainValue: GainNode {} not found", node_id);
                    }
                }

                AudioCmd::Connect { src, dst, resp } => {
                    let mut found = false;
                    for ctx in contexts.values_mut() {
                        ctx.connect(src, dst);
                        found = true;
                        break;
                    }
                    if found {
                        let _ = resp.send(Ok(()));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            "No context found",
                        )));
                    }
                }

                AudioCmd::Disconnect { node_id, dst, resp } => {
                    for ctx in contexts.values_mut() {
                        ctx.disconnect(node_id, dst);
                    }
                    let _ = resp.send(Ok(()));
                }

                // ==================== InnerAudioContext ====================
                AudioCmd::CreateInnerAudio { id } => {
                    if !inner_players.contains_key(&id) {
                        inner_players.insert(id, InnerAudioPlayer::new(id, channels));
                        tracing::debug!("Created InnerAudioContext {}", id);
                    }
                }

                AudioCmd::DestroyInnerAudio { id } => {
                    if inner_players.remove(&id).is_some() {
                        tracing::debug!("Destroyed InnerAudioContext {}", id);
                    }
                }

                AudioCmd::InnerAudioLoad { id, data, resp } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        match decoder::decode(&data) {
                            Ok(decoded) => {
                                // Resample to output device sample rate if needed
                                match resampler::resample_if_needed(decoded, sample_rate) {
                                    Ok(resampled) => {
                                        let info = InnerAudioInfo {
                                            duration: resampled.duration(),
                                            sample_rate: resampled.sample_rate,
                                            channels: resampled.channels,
                                        };
                                        player.load_audio(resampled);
                                        let _ = resp.send(Ok(info));
                                    }
                                    Err(e) => {
                                        let _ = resp.send(Err(e));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = resp.send(Err(e));
                            }
                        }
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("InnerAudioContext {} not found", id),
                        )));
                    }
                }

                AudioCmd::InnerAudioLoadUrl { id, url, resp } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        // Check cache first
                        if let Some(cached_audio) = audio_cache.get(&url) {
                            tracing::debug!("Cache hit for InnerAudioContext {}: {}", id, url);
                            player.load_cached(cached_audio);
                            let _ = resp.send(Ok(()));
                        } else {
                            // Start streaming download
                            let state = StreamingState::new();
                            let rx = streaming::start_streaming_download(
                                url.clone(),
                                state.clone(),
                                sample_rate,
                            );
                            player.start_streaming(url, rx, state);
                            let _ = resp.send(Ok(()));
                            tracing::debug!("Started streaming for InnerAudioContext {}: (cache miss)", id);
                        }
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("InnerAudioContext {} not found", id),
                        )));
                    }
                }

                AudioCmd::InnerAudioPlay { id } => {
                    tracing::trace!("InnerAudioPlay command: id={}", id);
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.play();
                    } else {
                        tracing::warn!("InnerAudioPlay: player {} not found", id);
                    }
                }

                AudioCmd::InnerAudioPause { id } => {
                    tracing::trace!("InnerAudioPause command: id={}", id);
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.pause();
                    } else {
                        tracing::warn!("InnerAudioPause: player {} not found", id);
                    }
                }

                AudioCmd::InnerAudioStop { id } => {
                    tracing::trace!("InnerAudioStop command: id={}", id);
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.stop();
                    } else {
                        tracing::warn!("InnerAudioStop: player {} not found", id);
                    }
                }

                AudioCmd::InnerAudioSeek { id, position } => {
                    tracing::trace!("InnerAudioSeek command: id={}, position={:.2}s", id, position);
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.shared.seek(position);
                    } else {
                        tracing::warn!("InnerAudioSeek: player {} not found", id);
                    }
                }

                AudioCmd::InnerAudioSetVolume { id, volume } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.shared.set_volume(volume);
                    }
                }

                AudioCmd::InnerAudioSetLoop { id, loop_enabled } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.shared.set_loop_enabled(loop_enabled);
                    }
                }

                AudioCmd::InnerAudioSetPlaybackRate { id, rate } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.shared.set_playback_rate(rate);
                    }
                }

                AudioCmd::InnerAudioSetAutoplay { id, autoplay } => {
                    if let Some(player) = inner_players.get_mut(&id) {
                        player.shared.set_autoplay(autoplay);
                    }
                }

                AudioCmd::InnerAudioGetState { id, resp } => {
                    if let Some(player) = inner_players.get(&id) {
                        let shared = &player.shared;
                        let state = InnerAudioState {
                            current_time: shared.current_time(),
                            duration: shared.duration(),
                            paused: shared.state() != PlaybackState::Playing,
                            volume: shared.volume(),
                            loop_enabled: shared.loop_enabled(),
                            playback_rate: shared.playback_rate(),
                            buffered: shared.is_loaded(),
                        };
                        let _ = resp.send(Ok(state));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("InnerAudioContext {} not found", id),
                        )));
                    }
                }

                AudioCmd::InnerAudioPollEvents { resp } => {
                    // Deprecated: polling is replaced by push mechanism
                    let _ = resp.send(Ok(Vec::new()));
                }
            }
        }

        // Poll streaming data for all players and cache completed downloads
        for player in inner_players.values_mut() {
            player.poll_stream();

            // Cache completed streaming audio
            if player.is_stream_complete() {
                if let Some(url) = player.loading_url().map(|s| s.to_string()) {
                    if let Some(audio) = player.take_streamed_audio() {
                        let cached = audio_cache.insert(url, audio);
                        // Update player to use cached reference
                        player.load_cached(cached);
                    }
                }
            }
        }

        // Collect events from all players and push to host
        for player in inner_players.values_mut() {
            for event in player.take_events() {
                let event_type = match event.event_type {
                    shared::protocol::audio_cmd::InnerAudioEventType::CanPlay => InnerAudioEventType::CanPlay,
                    shared::protocol::audio_cmd::InnerAudioEventType::Play => InnerAudioEventType::Play,
                    shared::protocol::audio_cmd::InnerAudioEventType::Pause => InnerAudioEventType::Pause,
                    shared::protocol::audio_cmd::InnerAudioEventType::Stop => InnerAudioEventType::Stop,
                    shared::protocol::audio_cmd::InnerAudioEventType::Ended => InnerAudioEventType::Ended,
                    shared::protocol::audio_cmd::InnerAudioEventType::Seeking => InnerAudioEventType::Seeking,
                    shared::protocol::audio_cmd::InnerAudioEventType::Seeked => InnerAudioEventType::Seeked,
                    shared::protocol::audio_cmd::InnerAudioEventType::TimeUpdate => InnerAudioEventType::TimeUpdate,
                    shared::protocol::audio_cmd::InnerAudioEventType::Error => InnerAudioEventType::Error,
                };
                tracing::trace!(
                    "Pushing InnerAudio event: id={}, type={:?}, time={:.2}s",
                    event.id, event_type, event.current_time
                );
                let _ = host_tx.try_send(HostCommand::InnerAudioEvent {
                    id: event.id,
                    event_type,
                    current_time: event.current_time,
                });
            }
        }

        // Check if any context or inner player is active
        let has_active_context = contexts
            .values()
            .any(|ctx| ctx.state() == AudioContextState::Running);

        let has_active_inner = inner_players.values().any(|p| p.is_active());

        if has_active_context || has_active_inner {
            // Check if callback signaled need for data (lightweight atomic check)
            if sync.check_and_clear() || output.needs_data() {
                // Fill buffer until it's above low watermark
                while output.needs_data() && output.available() >= buffer_size {
                    process_buffer.fill(0.0);

                    // Process WebAudio contexts
                    for ctx in contexts.values_mut() {
                        if ctx.state() == AudioContextState::Running {
                            ctx.process(&mut process_buffer);
                        }
                    }

                    // Process InnerAudioContext players
                    for player in inner_players.values_mut() {
                        player.process(&mut process_buffer);
                    }

                    output.write(&process_buffer);
                }
            }

            // Short sleep - balance between latency and CPU usage
            // ~5ms gives good responsiveness while keeping CPU low
            thread::sleep(Duration::from_millis(5));
        } else {
            // No active audio - longer sleep to save power
            thread::sleep(Duration::from_millis(50));
        }
    }
}
