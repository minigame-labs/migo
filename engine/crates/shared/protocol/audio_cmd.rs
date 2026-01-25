use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::error::EngineResult;

pub type AudioContextId = u32;
pub type AudioBufferId = u32;
pub type AudioNodeId = u32;
pub type InnerAudioId = u32;

/// Response sender for audio commands
pub type AudioResp<T> = oneshot::Sender<EngineResult<T>>;

/// Information about a decoded audio buffer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioBufferInfo {
    pub id: AudioBufferId,
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub length: u32, // number of sample frames
}

/// Playback state of an audio context
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioContextState {
    Suspended,
    Running,
    Closed,
}

/// Audio command protocol
pub enum AudioCmd {
    // ==================== Context ====================
    /// Create a new AudioContext
    CreateContext {
        sample_rate: Option<u32>,
        resp: AudioResp<AudioContextId>,
    },

    /// Close an AudioContext and release resources
    CloseContext {
        ctx_id: AudioContextId,
        resp: AudioResp<()>,
    },

    /// Get the current state of an AudioContext
    GetContextState {
        ctx_id: AudioContextId,
        resp: AudioResp<AudioContextState>,
    },

    /// Resume a suspended AudioContext
    ResumeContext {
        ctx_id: AudioContextId,
        resp: AudioResp<()>,
    },

    /// Suspend an AudioContext
    SuspendContext {
        ctx_id: AudioContextId,
        resp: AudioResp<()>,
    },

    // ==================== Buffer ====================
    /// Decode audio data into an AudioBuffer
    DecodeAudioData {
        ctx_id: AudioContextId,
        data: Vec<u8>,
        resp: AudioResp<AudioBufferInfo>,
    },

    /// Release an AudioBuffer
    ReleaseBuffer {
        buffer_id: AudioBufferId,
        resp: AudioResp<()>,
    },

    // ==================== Nodes ====================
    /// Create an AudioBufferSourceNode (JS provides node_id, fire-and-forget)
    CreateBufferSource {
        ctx_id: AudioContextId,
        node_id: AudioNodeId,
    },

    /// Set the buffer for an AudioBufferSourceNode
    SetBuffer {
        node_id: AudioNodeId,
        buffer_id: AudioBufferId,
        resp: AudioResp<()>,
    },

    /// Start playback of an AudioBufferSourceNode
    Start {
        node_id: AudioNodeId,
        when: f64,
        offset: f64,
        duration: Option<f64>,
        resp: AudioResp<()>,
    },

    /// Stop playback of an AudioBufferSourceNode
    Stop {
        node_id: AudioNodeId,
        when: f64,
        resp: AudioResp<()>,
    },

    /// Set loop property (fire-and-forget)
    SetLoop {
        node_id: AudioNodeId,
        loop_enabled: bool,
        loop_start: f64,
        loop_end: f64,
    },

    /// Set playback rate
    SetPlaybackRate {
        node_id: AudioNodeId,
        rate: f32,
        resp: AudioResp<()>,
    },

    /// Create a GainNode (JS provides node_id, fire-and-forget)
    CreateGain {
        ctx_id: AudioContextId,
        node_id: AudioNodeId,
    },

    /// Set gain value (fire-and-forget)
    SetGainValue {
        node_id: AudioNodeId,
        value: f32,
    },

    // ==================== Graph ====================
    /// Connect two nodes
    Connect {
        src: AudioNodeId,
        dst: AudioNodeId,
        resp: AudioResp<()>,
    },

    /// Disconnect a node from all outputs or a specific destination
    Disconnect {
        node_id: AudioNodeId,
        dst: Option<AudioNodeId>,
        resp: AudioResp<()>,
    },

    // ==================== Lifecycle ====================
    /// Shutdown the audio thread
    Shutdown,

    // ==================== InnerAudioContext ====================
    /// Create an InnerAudioContext (JS provides id, fire-and-forget)
    CreateInnerAudio {
        id: InnerAudioId,
    },

    /// Destroy an InnerAudioContext
    DestroyInnerAudio {
        id: InnerAudioId,
    },

    /// Load audio data into InnerAudioContext (full load mode)
    InnerAudioLoad {
        id: InnerAudioId,
        data: Vec<u8>,
        resp: AudioResp<InnerAudioInfo>,
    },

    /// Load audio from URL with streaming (edge-download-edge-play)
    InnerAudioLoadUrl {
        id: InnerAudioId,
        url: String,
        resp: AudioResp<()>,
    },

    /// Play InnerAudioContext
    InnerAudioPlay {
        id: InnerAudioId,
    },

    /// Pause InnerAudioContext
    InnerAudioPause {
        id: InnerAudioId,
    },

    /// Stop InnerAudioContext
    InnerAudioStop {
        id: InnerAudioId,
    },

    /// Seek InnerAudioContext to position (in seconds)
    InnerAudioSeek {
        id: InnerAudioId,
        position: f64,
    },

    /// Set InnerAudioContext volume (0.0 - 1.0)
    InnerAudioSetVolume {
        id: InnerAudioId,
        volume: f32,
    },

    /// Set InnerAudioContext loop
    InnerAudioSetLoop {
        id: InnerAudioId,
        loop_enabled: bool,
    },

    /// Set InnerAudioContext playback rate (0.5 - 2.0)
    InnerAudioSetPlaybackRate {
        id: InnerAudioId,
        rate: f32,
    },

    /// Set InnerAudioContext autoplay
    InnerAudioSetAutoplay {
        id: InnerAudioId,
        autoplay: bool,
    },

    /// Get InnerAudioContext current state
    InnerAudioGetState {
        id: InnerAudioId,
        resp: AudioResp<InnerAudioState>,
    },

    /// Poll for pending InnerAudioContext events
    InnerAudioPollEvents {
        resp: AudioResp<Vec<InnerAudioEvent>>,
    },
}

/// Information about loaded InnerAudioContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerAudioInfo {
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u32,
}

/// State of InnerAudioContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerAudioState {
    /// Current playback position in seconds
    pub current_time: f64,
    /// Total duration in seconds
    pub duration: f64,
    /// Whether audio is paused
    pub paused: bool,
    /// Current volume (0.0 - 1.0)
    pub volume: f32,
    /// Whether loop is enabled
    pub loop_enabled: bool,
    /// Current playback rate
    pub playback_rate: f32,
    /// Whether audio is loaded and ready to play
    pub buffered: bool,
}

/// Event types for InnerAudioContext
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InnerAudioEventType {
    CanPlay,
    Play,
    Pause,
    Stop,
    Ended,
    Seeking,
    Seeked,
    TimeUpdate,
    Error,
}

/// Event from InnerAudioContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerAudioEvent {
    /// ID of the InnerAudioContext that generated this event
    pub id: InnerAudioId,
    /// Type of event
    pub event_type: InnerAudioEventType,
    /// Current playback time when event occurred
    pub current_time: f64,
}
