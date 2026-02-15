use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::audio_cmd::{
    AudioBufferInfo, AudioCmd, AudioContextId, AudioContextState, AudioNodeId,
    InnerAudioId, InnerAudioInfo, InnerAudioState,
};
use shared::protocol::host_cmd::HostCommand;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{error, info};

use crate::cache::GlobalAudioCache;
use crate::context::AudioContext;
use crate::decoder;
use crate::inner_audio::{InnerAudioPlayer, PlaybackState};
use crate::nodes::{
    AnalyserNode, BiquadFilterNode, BiquadFilterType, ChannelMergerNode,
    ChannelSplitterNode, ConstantSourceNode, DelayNode, DynamicsCompressorNode,
    IIRFilterNode, OscillatorNode, OscillatorType, OversampleType,
    PannerNode, PanningModel, DistanceModel, WaveShaperNode,
};
use crate::output::AudioOutput;
use crate::resampler;
use crate::streaming::{self, StreamingState};

/// Reverse lookup from node_id to context_id for O(1) access.
/// This avoids iterating all contexts when looking up a node.
struct NodeContextIndex {
    node_to_ctx: HashMap<AudioNodeId, AudioContextId>,
}

impl NodeContextIndex {
    fn new() -> Self {
        Self {
            node_to_ctx: HashMap::with_capacity(64),
        }
    }

    #[inline]
    fn register(&mut self, node_id: AudioNodeId, ctx_id: AudioContextId) {
        self.node_to_ctx.insert(node_id, ctx_id);
    }

    #[inline]
    #[allow(dead_code)]
    fn unregister(&mut self, node_id: AudioNodeId) {
        self.node_to_ctx.remove(&node_id);
    }

    #[inline]
    fn get_context(&self, node_id: AudioNodeId) -> Option<AudioContextId> {
        self.node_to_ctx.get(&node_id).copied()
    }

    fn clear_context(&mut self, ctx_id: AudioContextId) {
        self.node_to_ctx.retain(|_, &mut v| v != ctx_id);
    }
}

/// Wakeup signal for the audio thread to avoid busy-waiting.
#[derive(Clone)]
pub struct AudioWakeup {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl AudioWakeup {
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Signal the audio thread to wake up.
    pub fn notify(&self) {
        let (lock, cvar) = &*self.inner;
        let mut signaled = lock.lock().unwrap();
        *signaled = true;
        cvar.notify_one();
    }

    /// Wait for a signal or timeout. Returns true if signaled.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.inner;
        let mut signaled = lock.lock().unwrap();
        if *signaled {
            *signaled = false;
            return true;
        }
        let result = cvar.wait_timeout(signaled, timeout).unwrap();
        signaled = result.0;
        let was_signaled = *signaled;
        *signaled = false;
        was_signaled || result.1.timed_out()
    }
}

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
            .name("Migo-AudioThread".into())
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

/// Calculate optimal process buffer size based on sample rate.
/// Higher sample rates need larger buffers for the same latency.
#[inline]
fn calculate_process_frames(sample_rate: u32) -> usize {
    // Target ~21ms of audio data
    // 48kHz * 0.021 ≈ 1008 frames, round to 1024 for alignment
    // 44.1kHz * 0.021 ≈ 926 frames, round to 1024
    // Higher rates like 96kHz would use 2048
    let target_ms = 21.0;
    let frames = (sample_rate as f32 * target_ms / 1000.0) as usize;
    // Round up to nearest power of 2 for better cache alignment
    frames.next_power_of_two().max(512).min(4096)
}

/// Audio thread main loop
fn run_audio_thread(
    mut rx: UnboundedReceiver<AudioCmd>,
    mut output: AudioOutput,
    host_tx: tokio::sync::mpsc::Sender<HostCommand>,
) {
    let sample_rate = output.sample_rate();
    let channels = output.channels();

    // Pre-allocate with reasonable capacity to avoid rehashing
    let mut contexts: HashMap<AudioContextId, AudioContext> = HashMap::with_capacity(4);
    let mut next_context_id: AudioContextId = 1;

    // Node-to-context index for O(1) lookup
    let mut node_index = NodeContextIndex::new();

    // InnerAudioContext players with pre-allocated capacity
    let mut inner_players: HashMap<InnerAudioId, InnerAudioPlayer> = HashMap::with_capacity(16);

    // MediaAudioPlayer: maps player_id -> list of InnerAudioContext source IDs
    let mut media_players: HashMap<u32, Vec<InnerAudioId>> = HashMap::with_capacity(4);

    // Global audio cache (64MB default)
    let audio_cache = GlobalAudioCache::new();

    // Audio processing buffer - dynamically sized based on sample rate
    let process_frames = calculate_process_frames(sample_rate);
    let buffer_size = process_frames * channels as usize;
    let mut process_buffer = vec![0.0f32; buffer_size];

    // Wakeup signal for efficient waiting
    let wakeup = AudioWakeup::new();

    // Get sync handle for callback-driven wakeup
    let mut sync = output.sync().clone();

    // Pause state: when true, skip audio processing but still handle commands.
    let mut paused = false;

    loop {
        // Process commands (non-blocking)
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                AudioCmd::Shutdown => {
                    return;
                }

                AudioCmd::PauseAll => {
                    if !paused {
                        paused = true;
                        info!("AudioThread paused");
                    }
                }

                AudioCmd::ResumeAll => {
                    if paused {
                        paused = false;
                        // If the audio stream died while paused (e.g. device
                        // disconnected during a phone call), recreate it now.
                        if !output.is_alive() {
                            match AudioOutput::new() {
                                Ok(new_output) => {
                                    sync = new_output.sync().clone();
                                    output = new_output;
                                    info!("AudioThread: audio output recreated after stream error");
                                }
                                Err(e) => {
                                    error!("AudioThread: failed to recreate audio output: {}", e);
                                }
                            }
                        }
                        info!("AudioThread resumed");
                    }
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
                        // Clear all nodes belonging to this context from the index
                        node_index.clear_context(ctx_id);
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
                        // Register node in index for fast lookup
                        node_index.register(node_id, ctx_id);
                    } else {
                        tracing::warn!("CreateBufferSource: AudioContext {} not found", ctx_id);
                    }
                }

                AudioCmd::SetBuffer {
                    node_id,
                    buffer_id,
                    resp,
                } => {
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.set_buffer(node_id, buffer_id))
                        .unwrap_or(false);

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
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.start_source(node_id, when, offset, duration))
                        .unwrap_or(false);

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
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.stop_source(node_id, when))
                        .unwrap_or(false);

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
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.set_loop(node_id, loop_enabled, loop_start, loop_end))
                        .unwrap_or(false);

                    if !found {
                        tracing::warn!("SetLoop: node {} not found", node_id);
                    }
                }

                AudioCmd::SetPlaybackRate { node_id, rate, resp } => {
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.set_playback_rate(node_id, rate))
                        .unwrap_or(false);

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
                        // Register node in index for fast lookup
                        node_index.register(node_id, ctx_id);
                    } else {
                        tracing::warn!("CreateGain: AudioContext {} not found", ctx_id);
                    }
                }

                AudioCmd::SetGainValue { node_id, value } => {
                    // Use index for O(1) context lookup
                    let found = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| ctx.set_gain(node_id, value))
                        .unwrap_or(false);

                    if !found {
                        tracing::warn!("SetGainValue: GainNode {} not found", node_id);
                    }
                }

                // ==================== Phase 2 Nodes ====================
                AudioCmd::CreateOscillator { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(OscillatorNode::new(node_id)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::SetOscillatorType { node_id, osc_type } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<OscillatorNode, _>(node_id, |osc| {
                                osc.set_type(OscillatorType::from_str(&osc_type));
                            });
                        }
                    }
                }

                AudioCmd::StartOscillator { node_id, when } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<OscillatorNode, _>(node_id, |osc| {
                                osc.start(when);
                            });
                        }
                    }
                }

                AudioCmd::StopOscillator { node_id, when } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<OscillatorNode, _>(node_id, |osc| {
                                osc.stop(when);
                            });
                        }
                    }
                }

                AudioCmd::CreateDelay { ctx_id, node_id, max_delay_time } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let sr = ctx.sample_rate();
                        let ch = ctx.channels();
                        ctx.add_node(Box::new(DelayNode::new(node_id, max_delay_time, sr, ch)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::CreateBiquadFilter { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let ch = ctx.channels();
                        ctx.add_node(Box::new(BiquadFilterNode::new(node_id, ch)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::SetBiquadFilterType { node_id, filter_type } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<BiquadFilterNode, _>(node_id, |filt| {
                                filt.set_type(BiquadFilterType::from_str(&filter_type));
                            });
                        }
                    }
                }

                AudioCmd::CreateWaveShaper { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(WaveShaperNode::new(node_id)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::SetWaveShaperCurve { node_id, curve } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<WaveShaperNode, _>(node_id, |ws| {
                                ws.set_curve(curve);
                            });
                        }
                    }
                }

                AudioCmd::SetWaveShaperOversample { node_id, oversample } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<WaveShaperNode, _>(node_id, |ws| {
                                ws.set_oversample(OversampleType::from_str(&oversample));
                            });
                        }
                    }
                }

                AudioCmd::CreateAnalyser { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let ch = ctx.channels();
                        ctx.add_node(Box::new(AnalyserNode::new(node_id, ch)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::SetAnalyserFftSize { node_id, fft_size } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<AnalyserNode, _>(node_id, |an| {
                                an.set_fft_size(fft_size as usize);
                            });
                        }
                    }
                }

                AudioCmd::GetAnalyserByteTimeDomainData { node_id, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            ctx.with_node_typed::<AnalyserNode, _>(node_id, |an| {
                                an.get_byte_time_domain_data()
                            })
                        });
                    match result {
                        Some(data) => { let _ = resp.send(Ok(data)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("AnalyserNode {} not found", node_id),
                            )));
                        }
                    }
                }

                AudioCmd::GetAnalyserFloatTimeDomainData { node_id, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            ctx.with_node_typed::<AnalyserNode, _>(node_id, |an| {
                                an.get_float_time_domain_data()
                            })
                        });
                    match result {
                        Some(data) => { let _ = resp.send(Ok(data)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("AnalyserNode {} not found", node_id),
                            )));
                        }
                    }
                }

                // ==================== Phase 3 Nodes ====================
                AudioCmd::CreateDynamicsCompressor { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let ch = ctx.channels();
                        ctx.add_node(Box::new(DynamicsCompressorNode::new(node_id, ch)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::CreatePanner { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(PannerNode::new(node_id)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::SetPanningModel { node_id, model } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<PannerNode, _>(node_id, |p| {
                                p.set_panning_model(PanningModel::from_str(&model));
                            });
                        }
                    }
                }

                AudioCmd::SetDistanceModel { node_id, model } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<PannerNode, _>(node_id, |p| {
                                p.set_distance_model(DistanceModel::from_str(&model));
                            });
                        }
                    }
                }

                AudioCmd::SetPannerScalar { node_id, prop, value } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<PannerNode, _>(node_id, |p| {
                                match prop.as_str() {
                                    "refDistance" => p.set_ref_distance(value),
                                    "maxDistance" => p.set_max_distance(value),
                                    "rolloffFactor" => p.set_rolloff_factor(value),
                                    "coneInnerAngle" => p.set_cone_inner_angle(value),
                                    "coneOuterAngle" => p.set_cone_outer_angle(value),
                                    "coneOuterGain" => p.set_cone_outer_gain(value),
                                    _ => {}
                                }
                            });
                        }
                    }
                }

                AudioCmd::CreateChannelMerger { ctx_id, node_id, number_of_inputs } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(ChannelMergerNode::new(node_id, number_of_inputs)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::CreateChannelSplitter { ctx_id, node_id, number_of_outputs } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(ChannelSplitterNode::new(node_id, number_of_outputs)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::CreateConstantSource { ctx_id, node_id } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        ctx.add_node(Box::new(ConstantSourceNode::new(node_id)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::StartConstantSource { node_id, when } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<ConstantSourceNode, _>(node_id, |cs| {
                                cs.start(when);
                            });
                        }
                    }
                }

                AudioCmd::StopConstantSource { node_id, when } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.with_node_typed::<ConstantSourceNode, _>(node_id, |cs| {
                                cs.stop(when);
                            });
                        }
                    }
                }

                AudioCmd::CreateIIRFilter { ctx_id, node_id, feedforward, feedback } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let ch = ctx.channels();
                        ctx.add_node(Box::new(IIRFilterNode::new(node_id, feedforward, feedback, ch)));
                        node_index.register(node_id, ctx_id);
                    }
                }

                AudioCmd::Connect { src, dst, resp } => {
                    // Use index to find the context containing the source node
                    let found = node_index
                        .get_context(src)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .map(|ctx| {
                            ctx.connect(src, dst);
                            true
                        })
                        .unwrap_or(false);

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
                    // Use index for O(1) context lookup
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.disconnect(node_id, dst);
                        }
                    }
                    let _ = resp.send(Ok(()));
                }

                // ==================== Frequency Response & Analysis ====================
                AudioCmd::GetFrequencyResponse { node_id, frequencies, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            let sr = ctx.sample_rate() as f64;
                            // Try BiquadFilterNode first
                            if let Some(r) = ctx.with_node_typed::<BiquadFilterNode, _>(node_id, |n| {
                                n.get_frequency_response(sr, &frequencies)
                            }) {
                                return Some(r);
                            }
                            // Try IIRFilterNode
                            ctx.with_node_typed::<IIRFilterNode, _>(node_id, |n| {
                                n.get_frequency_response(sr, &frequencies)
                            })
                        });
                    match result {
                        Some(data) => { let _ = resp.send(Ok(data)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("Filter node {} not found", node_id),
                            )));
                        }
                    }
                }

                AudioCmd::GetReduction { node_id, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            ctx.with_node_typed::<DynamicsCompressorNode, _>(node_id, |n| {
                                n.reduction()
                            })
                        });
                    match result {
                        Some(val) => { let _ = resp.send(Ok(val)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("DynamicsCompressorNode {} not found", node_id),
                            )));
                        }
                    }
                }

                AudioCmd::GetAnalyserByteFrequencyData { node_id, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            ctx.with_node_typed::<AnalyserNode, _>(node_id, |an| {
                                an.get_byte_frequency_data()
                            })
                        });
                    match result {
                        Some(data) => { let _ = resp.send(Ok(data)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("AnalyserNode {} not found", node_id),
                            )));
                        }
                    }
                }

                AudioCmd::GetAnalyserFloatFrequencyData { node_id, resp } => {
                    let result = node_index
                        .get_context(node_id)
                        .and_then(|ctx_id| contexts.get_mut(&ctx_id))
                        .and_then(|ctx| {
                            ctx.with_node_typed::<AnalyserNode, _>(node_id, |an| {
                                an.get_float_frequency_data()
                            })
                        });
                    match result {
                        Some(data) => { let _ = resp.send(Ok(data)); }
                        None => {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("AnalyserNode {} not found", node_id),
                            )));
                        }
                    }
                }

                // ==================== AudioParam Automation ====================
                AudioCmd::AudioParamSetValueAtTime { node_id, param_name, value, time } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.param_set_value_at_time(node_id, &param_name, value, time);
                        }
                    }
                }

                AudioCmd::AudioParamLinearRamp { node_id, param_name, value, end_time } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.param_linear_ramp(node_id, &param_name, value, end_time);
                        }
                    }
                }

                AudioCmd::AudioParamExponentialRamp { node_id, param_name, value, end_time } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.param_exponential_ramp(node_id, &param_name, value, end_time);
                        }
                    }
                }

                AudioCmd::AudioParamSetTarget { node_id, param_name, target, start_time, time_constant } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.param_set_target(node_id, &param_name, target, start_time, time_constant);
                        }
                    }
                }

                AudioCmd::AudioParamCancelScheduled { node_id, param_name, cancel_time } => {
                    if let Some(ctx_id) = node_index.get_context(node_id) {
                        if let Some(ctx) = contexts.get_mut(&ctx_id) {
                            ctx.param_cancel_scheduled(node_id, &param_name, cancel_time);
                        }
                    }
                }

                // ==================== Buffer Data Access ====================
                AudioCmd::CreateBuffer { ctx_id, channels, length, sample_rate: buf_rate, resp } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        let id = ctx.create_empty_buffer(channels, length, buf_rate);
                        let _ = resp.send(Ok(AudioBufferInfo {
                            id,
                            duration: length as f64 / buf_rate as f64,
                            sample_rate: buf_rate,
                            channels,
                            length,
                        }));
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::GetChannelData { ctx_id, buffer_id, channel, resp } => {
                    if let Some(ctx) = contexts.get(&ctx_id) {
                        if let Some(data) = ctx.get_channel_data(buffer_id, channel) {
                            let _ = resp.send(Ok(data));
                        } else {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("Buffer {} channel {} not found", buffer_id, channel),
                            )));
                        }
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                AudioCmd::CopyToChannel { ctx_id, buffer_id, data, channel, start, resp } => {
                    if let Some(ctx) = contexts.get_mut(&ctx_id) {
                        if ctx.copy_to_channel(buffer_id, &data, channel, start) {
                            let _ = resp.send(Ok(()));
                        } else {
                            let _ = resp.send(Err(EngineError::from_detail(
                                ErrorCode::NotFound,
                                format!("Buffer {} channel {} not found", buffer_id, channel),
                            )));
                        }
                    } else {
                        let _ = resp.send(Err(EngineError::from_detail(
                            ErrorCode::NotFound,
                            format!("AudioContext {} not found", ctx_id),
                        )));
                    }
                }

                // ==================== Global Audio Options ====================
                AudioCmd::SetInnerAudioOption { mix_with_other, obey_mute_switch, speaker_on, resp } => {
                    // Store settings. Actual platform behavior (Android AudioManager)
                    // is handled at the platform layer. The audio thread acknowledges.
                    tracing::debug!(
                        "SetInnerAudioOption: mix={}, mute_switch={}, speaker={}",
                        mix_with_other, obey_mute_switch, speaker_on
                    );
                    let _ = resp.send(Ok(()));
                }

                AudioCmd::GetAvailableAudioSources { resp } => {
                    // Return default audio sources. Platform-specific sources
                    // (e.g. Bluetooth, USB) require Android AudioManager queries.
                    let sources = vec![
                        "auto".to_string(),
                        "buildInMic".to_string(),
                        "mic".to_string(),
                    ];
                    let _ = resp.send(Ok(sources));
                }

                // ==================== MediaAudioPlayer ====================
                AudioCmd::CreateMediaAudioPlayer { id } => {
                    media_players.insert(id, Vec::new());
                    tracing::debug!("Created MediaAudioPlayer {}", id);
                }

                AudioCmd::MediaAudioPlayerAddSource { player_id, source_id } => {
                    if let Some(sources) = media_players.get_mut(&player_id) {
                        if !sources.contains(&source_id) {
                            sources.push(source_id);
                        }
                    }
                }

                AudioCmd::MediaAudioPlayerRemoveSource { player_id, source_id } => {
                    if let Some(sources) = media_players.get_mut(&player_id) {
                        sources.retain(|&id| id != source_id);
                    }
                }

                AudioCmd::MediaAudioPlayerStart { player_id } => {
                    if let Some(sources) = media_players.get(&player_id) {
                        for &source_id in sources {
                            if let Some(player) = inner_players.get_mut(&source_id) {
                                player.play();
                            }
                        }
                    }
                }

                AudioCmd::MediaAudioPlayerStop { player_id } => {
                    if let Some(sources) = media_players.get(&player_id) {
                        for &source_id in sources {
                            if let Some(player) = inner_players.get_mut(&source_id) {
                                player.stop();
                            }
                        }
                    }
                }

                AudioCmd::MediaAudioPlayerDestroy { player_id } => {
                    media_players.remove(&player_id);
                    tracing::debug!("Destroyed MediaAudioPlayer {}", player_id);
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

        // When paused, skip all audio processing and wait for commands.
        if paused {
            wakeup.wait_timeout(Duration::from_millis(100));
            continue;
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

        // Collect events from all players and push to host.
        // audio_cmd::InnerAudioEventType is re-exported from host_cmd — same type, no mapping.
        for player in inner_players.values_mut() {
            for event in player.take_events() {
                tracing::trace!(
                    "Pushing InnerAudio event: id={}, type={:?}, time={:.2}s",
                    event.id, event.event_type, event.current_time
                );
                let _ = host_tx.try_send(HostCommand::InnerAudioEvent {
                    id: event.id,
                    event_type: event.event_type,
                    current_time: event.current_time,
                });
            }
        }

        // If the audio stream died (e.g. device disconnected during a call),
        // try to recreate it so audio resumes without waiting for ResumeAll.
        if !output.is_alive() {
            match AudioOutput::new() {
                Ok(new_output) => {
                    sync = new_output.sync().clone();
                    output = new_output;
                    info!("AudioThread: audio output recovered after stream error");
                }
                Err(e) => {
                    error!("AudioThread: failed to recover audio output: {}", e);
                    // Wait before retrying to avoid busy-loop on persistent errors
                    wakeup.wait_timeout(Duration::from_secs(1));
                    continue;
                }
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

            // Use condvar-based wait instead of busy-sleep for better efficiency.
            // This reduces CPU usage while maintaining low latency when audio
            // callback or commands arrive.
            wakeup.wait_timeout(Duration::from_millis(5));
        } else {
            // No active audio - use longer wait to save power.
            // The wakeup signal ensures we respond quickly when audio starts.
            wakeup.wait_timeout(Duration::from_millis(50));
        }
    }
}
