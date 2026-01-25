use deno_core::{JsBuffer, OpState, op2};
use shared::{
    error::{EngineError, ErrorCode},
    op_state::HostOpState,
    protocol::audio_cmd::{
        AudioBufferId, AudioBufferInfo, AudioCmd, AudioContextId, AudioNodeId,
        InnerAudioId, InnerAudioInfo, InnerAudioState,
    },
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tracing;

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum AudioError {
    #[class("AudioError")]
    #[error("{0}")]
    Message(String),
}

impl From<&str> for AudioError {
    #[inline]
    fn from(value: &str) -> Self {
        AudioError::Message(value.to_string())
    }
}

impl From<String> for AudioError {
    #[inline]
    fn from(value: String) -> Self {
        AudioError::Message(value)
    }
}

impl From<ErrorCode> for AudioError {
    #[inline]
    fn from(e: ErrorCode) -> Self {
        AudioError::Message(e.default_message().to_string())
    }
}

impl From<EngineError> for AudioError {
    #[inline]
    fn from(e: EngineError) -> Self {
        match &e.detail {
            Some(d) => AudioError::Message(format!("[{:?}] {} ({})", e.code, e.msg, d)),
            None => AudioError::Message(format!("[{:?}] {}", e.code, e.msg)),
        }
    }
}

#[inline]
fn audio_err(msg: impl Into<String>) -> AudioError {
    AudioError::Message(msg.into())
}

#[inline]
fn get_audio_tx(state: Rc<RefCell<OpState>>) -> UnboundedSender<AudioCmd> {
    let st = state.borrow();
    st.borrow::<HostOpState>().audio_tx.clone()
}

// ============================================================================
// Context Operations
// ============================================================================

#[op2(async)]
#[smi]
pub async fn op_audio_create_context(
    state: Rc<RefCell<OpState>>,
    #[smi] sample_rate: u32,
) -> Result<AudioContextId, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    let sample_rate_opt = if sample_rate == 0 {
        None
    } else {
        Some(sample_rate)
    };

    tx.send(AudioCmd::CreateContext {
        sample_rate: sample_rate_opt,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[op2(async)]
pub async fn op_audio_close_context(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::CloseContext {
        ctx_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// Buffer Operations
// ============================================================================

#[op2(async)]
#[serde]
pub async fn op_audio_decode_audio_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[buffer] data: JsBuffer,
) -> Result<AudioBufferInfo, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::DecodeAudioData {
        ctx_id,
        data: data.to_vec(),
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// BufferSourceNode Operations
// ============================================================================

/// Create a buffer source node with JS-provided node_id (fire and forget)
#[op2(fast)]
pub fn op_audio_create_buffer_source(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::CreateBufferSource { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(async)]
pub async fn op_audio_set_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[smi] buffer_id: AudioBufferId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::SetBuffer {
        node_id,
        buffer_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[op2(async)]
pub async fn op_audio_start(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
    offset: f64,
    duration: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    // Use NaN or negative to indicate no duration limit
    let duration_opt = if duration.is_nan() || duration < 0.0 {
        None
    } else {
        Some(duration)
    };

    tx.send(AudioCmd::Start {
        node_id,
        when,
        offset,
        duration: duration_opt,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[op2(async)]
pub async fn op_audio_stop(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::Stop {
        node_id,
        when,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Set loop property (fire and forget)
#[op2(fast)]
pub fn op_audio_set_loop(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::SetLoop {
        node_id,
        loop_enabled,
        loop_start,
        loop_end,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// GainNode Operations
// ============================================================================

/// Create a gain node with JS-provided node_id (fire and forget)
#[op2(fast)]
pub fn op_audio_create_gain(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::CreateGain { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Set gain value (fire and forget)
#[op2(fast)]
pub fn op_audio_set_gain_value(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    value: f32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::SetGainValue { node_id, value })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Graph Operations
// ============================================================================

#[op2(async)]
pub async fn op_audio_connect(
    state: Rc<RefCell<OpState>>,
    #[smi] src: AudioNodeId,
    #[smi] dst: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::Connect {
        src,
        dst,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[op2(async)]
pub async fn op_audio_disconnect(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::Disconnect {
        node_id,
        dst: None,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// InnerAudioContext Operations
// ============================================================================

/// Create an InnerAudioContext with JS-provided id (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_create(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateInnerAudio { id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Destroy an InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_destroy(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::DestroyInnerAudio { id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Load audio data into InnerAudioContext (full load mode - deprecated, use op_inner_audio_load_url)
#[op2(async)]
#[serde]
pub async fn op_inner_audio_load(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    #[buffer] data: JsBuffer,
) -> Result<InnerAudioInfo, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::InnerAudioLoad {
        id,
        data: data.to_vec(),
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Check if a string is an HTTP/HTTPS URL
#[inline]
fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Resolve a relative path against code_dir
#[inline]
fn resolve_path(code_dir: Option<&str>, path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return path.to_string();
    }
    match code_dir {
        Some(base) if !base.is_empty() => {
            let mut full = PathBuf::from(base);
            full.push(path);
            full.to_string_lossy().into_owned()
        }
        _ => path.to_string(),
    }
}

/// Load audio from URL or local path
/// - HTTP/HTTPS URLs: streaming download (edge-download-edge-play)
/// - Local paths: read file and load synchronously
#[op2(async)]
pub async fn op_inner_audio_load_url(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    #[string] src: String,
) -> Result<(), AudioError> {
    if is_http_url(&src) {
        // HTTP URL - use streaming download
        let tx = get_audio_tx(state);
        let (resp_tx, resp_rx) = oneshot::channel();

        tx.send(AudioCmd::InnerAudioLoadUrl {
            id,
            url: src,
            resp: resp_tx,
        })
        .map_err(|_| audio_err("Audio thread disconnected"))?;

        resp_rx
            .await
            .map_err(|_| audio_err("Response channel closed"))?
            .map_err(AudioError::from)
    } else {
        // Local file - resolve path and read file
        let (tx, code_dir) = {
            let st = state.borrow();
            let host = st.borrow::<HostOpState>();
            (host.audio_tx.clone(), host.code_dir.clone())
        };

        let full_path = resolve_path(code_dir.as_deref(), &src);
        tracing::debug!("Loading local audio file: {}", full_path);

        // Read file asynchronously
        let data = tokio::fs::read(&full_path)
            .await
            .map_err(|e| audio_err(&format!("Failed to read audio file '{}': {}", full_path, e)))?;

        let (resp_tx, resp_rx) = oneshot::channel();

        tx.send(AudioCmd::InnerAudioLoad {
            id,
            data,
            resp: resp_tx,
        })
        .map_err(|_| audio_err("Audio thread disconnected"))?;

        // Wait for load, but we don't need the InnerAudioInfo here
        resp_rx
            .await
            .map_err(|_| audio_err("Response channel closed"))?
            .map_err(AudioError::from)?;

        Ok(())
    }
}

/// Play InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_play(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioPlay { id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Pause InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_pause(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioPause { id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Stop InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_stop(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioStop { id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Seek InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_seek(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    position: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioSeek { id, position })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Set volume (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_set_volume(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    volume: f32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioSetVolume { id, volume })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Set loop (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_set_loop(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    loop_enabled: bool,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioSetLoop { id, loop_enabled })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Set playback rate (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_set_playback_rate(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    rate: f32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioSetPlaybackRate { id, rate })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Set autoplay (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_set_autoplay(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    autoplay: bool,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioSetAutoplay { id, autoplay })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Get current state
#[op2(async)]
#[serde]
pub async fn op_inner_audio_get_state(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<InnerAudioState, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::InnerAudioGetState { id, resp: resp_tx })
        .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

