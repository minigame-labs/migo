use bytemuck;
use deno_core::{op2, JsBuffer, OpState};
use shared::{
    error::{EngineError, ErrorCode},
    op_state::{AudioSender, HostOpState},
    protocol::audio_cmd::{
        AudioBufferId, AudioBufferInfo, AudioCmd, AudioContextId, AudioNodeId, InnerAudioId,
        InnerAudioInfo, InnerAudioState,
    },
    vfs::{FileOp, VirtualFS},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use tokio::sync::oneshot;
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

impl From<shared::protocol::error::ServiceError> for AudioError {
    #[inline]
    fn from(e: shared::protocol::error::ServiceError) -> Self {
        AudioError::Message(e.message)
    }
}

#[inline]
fn audio_err(msg: impl Into<String>) -> AudioError {
    AudioError::Message(msg.into())
}

#[inline]
fn get_audio_tx(state: Rc<RefCell<OpState>>) -> AudioSender {
    let st = state.borrow();
    st.borrow::<HostOpState>().audio_tx.clone()
}

// ============================================================================
// Context Operations
// ============================================================================

#[op2(async(lazy), fast)]
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

#[op2(async(lazy), fast)]
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

/// Resume a suspended AudioContext
#[op2(async(lazy), fast)]
pub async fn op_audio_resume_context(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::ResumeContext {
        ctx_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Suspend an AudioContext
#[op2(async(lazy), fast)]
pub async fn op_audio_suspend_context(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::SuspendContext {
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

#[op2(async(lazy))]
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
        data: std::sync::Arc::new(data.to_vec()),
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

#[op2(async(lazy), fast)]
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

#[op2(async(lazy), fast)]
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

#[op2(async(lazy), fast)]
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

#[op2(async(lazy), fast)]
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

#[op2(async(lazy), fast)]
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
// AudioParam Automation Operations (all fire-and-forget)
// ============================================================================

#[op2(fast)]
pub fn op_audio_param_set_value_at_time(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: String,
    value: f32,
    time: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::AudioParamSetValueAtTime {
        node_id,
        param_name,
        value,
        time,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_param_linear_ramp(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: String,
    value: f32,
    end_time: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::AudioParamLinearRamp {
        node_id,
        param_name,
        value,
        end_time,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_param_exponential_ramp(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: String,
    value: f32,
    end_time: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::AudioParamExponentialRamp {
        node_id,
        param_name,
        value,
        end_time,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_param_set_target(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: String,
    target: f32,
    start_time: f64,
    time_constant: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::AudioParamSetTarget {
        node_id,
        param_name,
        target,
        start_time,
        time_constant,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_param_cancel_scheduled(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: String,
    cancel_time: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::AudioParamCancelScheduled {
        node_id,
        param_name,
        cancel_time,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 2: OscillatorNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_oscillator(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateOscillator { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_oscillator_type(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] osc_type: String,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetOscillatorType { node_id, osc_type })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_start_oscillator(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StartOscillator { node_id, when })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_stop_oscillator(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StopOscillator { node_id, when })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 2: DelayNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_delay(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    max_delay_time: f32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateDelay {
        ctx_id,
        node_id,
        max_delay_time,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 2: BiquadFilterNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_biquad_filter(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateBiquadFilter { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_biquad_filter_type(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] filter_type: String,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetBiquadFilterType {
        node_id,
        filter_type,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 2: WaveShaperNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_wave_shaper(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateWaveShaper { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2]
pub fn op_audio_set_wave_shaper_curve(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[buffer] curve_bytes: Option<JsBuffer>,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let curve = curve_bytes.map(|buf| {
        buf.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect::<Vec<f32>>()
    });
    tx.send(AudioCmd::SetWaveShaperCurve { node_id, curve })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_wave_shaper_oversample(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] oversample: String,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetWaveShaperOversample {
        node_id,
        oversample,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 2: AnalyserNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_analyser(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateAnalyser { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_analyser_fft_size(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[smi] fft_size: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetAnalyserFftSize { node_id, fft_size })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(async(lazy), fast)]
#[buffer]
pub async fn op_audio_analyser_byte_time_domain(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<Vec<u8>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();
    tx.send(AudioCmd::GetAnalyserByteTimeDomainData {
        node_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;
    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[op2(async(lazy), fast)]
#[buffer]
pub async fn op_audio_analyser_float_time_domain(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<Vec<u8>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();
    tx.send(AudioCmd::GetAnalyserFloatTimeDomainData {
        node_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;
    let data = resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)?;
    // Convert Vec<f32> to raw bytes
    Ok(data.iter().flat_map(|f| f.to_le_bytes()).collect())
}

// ============================================================================
// Phase 3: DynamicsCompressorNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_dynamics_compressor(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateDynamicsCompressor { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 3: PannerNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_panner(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreatePanner { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_panning_model(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] model: String,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetPanningModel { node_id, model })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_distance_model(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] model: String,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetDistanceModel { node_id, model })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_set_panner_scalar(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] prop: String,
    value: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetPannerScalar {
        node_id,
        prop,
        value,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 3: ChannelMerger/Splitter Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_channel_merger(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[smi] number_of_inputs: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateChannelMerger {
        ctx_id,
        node_id,
        number_of_inputs,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_create_channel_splitter(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[smi] number_of_outputs: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateChannelSplitter {
        ctx_id,
        node_id,
        number_of_outputs,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 3: ConstantSourceNode Operations
// ============================================================================

#[op2(fast)]
pub fn op_audio_create_constant_source(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateConstantSource { ctx_id, node_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_start_constant_source(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StartConstantSource { node_id, when })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

#[op2(fast)]
pub fn op_audio_stop_constant_source(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StopConstantSource { node_id, when })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// Phase 3: IIRFilterNode Operations
// ============================================================================

#[op2]
pub fn op_audio_create_iir_filter(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[serde] feedforward: Vec<f64>,
    #[serde] feedback: Vec<f64>,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateIIRFilter {
        ctx_id,
        node_id,
        feedforward,
        feedback,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

// ============================================================================
// AudioBuffer Data Access Operations
// ============================================================================

/// Create an empty audio buffer
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_audio_create_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] channels: u32,
    #[smi] length: u32,
    #[smi] sample_rate: u32,
) -> Result<AudioBufferInfo, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::CreateBuffer {
        ctx_id,
        channels,
        length,
        sample_rate,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Get channel data from a buffer (returns raw f32 bytes)
#[op2(async(lazy), fast)]
#[buffer]
pub async fn op_audio_get_channel_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] buffer_id: AudioBufferId,
    #[smi] channel: u32,
) -> Result<Vec<u8>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetChannelData {
        ctx_id,
        buffer_id,
        channel,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    let data = resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)?;

    // Zero-copy reinterpret Vec<f32> as Vec<u8> via bytemuck.
    // This avoids an O(n) per-float collect and eliminates a heap allocation.
    Ok(bytemuck::cast_vec(data))
}

/// Get all channel data in one round-trip as a flat `Uint8Array`.
///
/// Layout: `[u32_le ch_count | u32_le frames_per_ch | ch0 f32s | ch1 f32s | ...]`
///
/// The 8-byte header carries metadata; the rest is raw LE f32 samples.
/// JS slices by stride: `Float32Array(buf, 8 + ch * frames * 4, frames)`.
///
/// `#[buffer]` ensures the result arrives as a real `Uint8Array` in JS
/// (not a JSON-serialized number array), preserving `.buffer` / `.byteOffset`.
#[op2(async(lazy),fast)]
#[buffer]
pub async fn op_audio_get_all_channel_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] buffer_id: AudioBufferId,
) -> Result<Vec<u8>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetAllChannelData {
        ctx_id,
        buffer_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    let channels = resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)?;

    let ch_count = channels.len() as u32;
    let frames_per_ch = channels.first().map_or(0, |c| c.len()) as u32;

    // 8-byte header + all channel f32 samples.
    let total_floats: usize = channels.iter().map(|c| c.len()).sum();
    let mut buf = Vec::with_capacity(8 + total_floats * 4);
    buf.extend_from_slice(&ch_count.to_le_bytes());
    buf.extend_from_slice(&frames_per_ch.to_le_bytes());
    for ch in channels {
        buf.extend_from_slice(bytemuck::cast_slice(&ch));
    }
    Ok(buf)
}

/// Copy data to a buffer channel (sync write to native buffer)
#[op2(async(lazy))]
pub async fn op_audio_copy_to_channel(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] buffer_id: AudioBufferId,
    #[buffer] data_bytes: JsBuffer,
    #[smi] channel: u32,
    #[smi] start: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    // Convert raw bytes to Vec<f32>
    let data: Vec<f32> = data_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    tx.send(AudioCmd::CopyToChannel {
        ctx_id,
        buffer_id,
        data,
        channel,
        start,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// Frequency Response & Analysis
// ============================================================================

/// Get frequency response from BiquadFilterNode or IIRFilterNode.
/// Returns (magResponse, phaseResponse) as interleaved bytes.
#[op2(async(lazy))]
#[serde]
pub async fn op_audio_get_frequency_response(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[serde] frequencies: Vec<f32>,
) -> Result<(Vec<f32>, Vec<f32>), AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetFrequencyResponse {
        node_id,
        frequencies,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Get current reduction value from DynamicsCompressorNode
#[op2(async(lazy), fast)]
pub async fn op_audio_get_reduction(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<f32, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetReduction {
        node_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Get byte frequency data from AnalyserNode (FFT output, 0-255)
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_audio_analyser_byte_frequency(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<Vec<u8>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetAnalyserByteFrequencyData {
        node_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Get float frequency data from AnalyserNode (FFT output, dB values)
#[op2(async(lazy), fast)]
#[serde]
pub async fn op_audio_analyser_float_frequency(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
) -> Result<Vec<f32>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetAnalyserFloatFrequencyData {
        node_id,
        resp: resp_tx,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// Global Audio Options
// ============================================================================

/// Set inner audio options via platform AudioManager.
/// Routes through DeviceServices for platform-specific audio configuration:
/// - mixWithOther: Android audio focus behavior (duck vs abandon)
/// - obeyMuteSwitch: Respect device ringer/mute mode
/// - speakerOn: Route audio output to speaker
#[op2(fast)]
pub fn op_audio_set_inner_audio_option(
    state: &mut OpState,
    mix_with_other: bool,
    obey_mute_switch: bool,
    speaker_on: bool,
) -> Result<(), AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(audio) = services.audio_platform() {
            return audio
                .set_inner_audio_option(mix_with_other, obey_mute_switch, speaker_on)
                .map_err(AudioError::from);
        }
    }
    Err(audio_err("setInnerAudioOption:fail not supported"))
}

/// Get available audio input sources from platform.
/// Queries Android AudioManager/MediaRecorder for supported audio sources.
/// Returns source identifiers matching RecorderManager.start() audioSource param.
#[op2]
#[serde]
pub fn op_audio_get_available_audio_sources(
    state: &mut OpState,
) -> Result<Vec<String>, AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(audio) = services.audio_platform() {
            return audio
                .get_available_audio_sources()
                .map_err(AudioError::from);
        }
    }
    Err(audio_err("getAvailableAudioSources:fail not supported"))
}

// ============================================================================
// Recorder Operations
// ============================================================================

/// Start recording with the given options (JSON string).
/// Routes through DeviceServices RecorderService.
#[op2(fast)]
pub fn op_recorder_start(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(recorder) = services.recorder() {
            return recorder.start(options_json).map_err(AudioError::from);
        }
    }
    Err(audio_err("recorderManager.start:fail not supported"))
}

/// Pause recording.
#[op2(fast)]
pub fn op_recorder_pause(state: &mut OpState) -> Result<(), AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(recorder) = services.recorder() {
            return recorder.pause().map_err(AudioError::from);
        }
    }
    Err(audio_err("recorderManager.pause:fail not supported"))
}

/// Resume recording after pause.
#[op2(fast)]
pub fn op_recorder_resume(state: &mut OpState) -> Result<(), AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(recorder) = services.recorder() {
            return recorder.resume().map_err(AudioError::from);
        }
    }
    Err(audio_err("recorderManager.resume:fail not supported"))
}

/// Stop recording. Results delivered asynchronously via RecorderEvent.
#[op2(fast)]
pub fn op_recorder_stop(state: &mut OpState) -> Result<(), AudioError> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(recorder) = services.recorder() {
            return recorder.stop().map_err(AudioError::from);
        }
    }
    Err(audio_err("recorderManager.stop:fail not supported"))
}

// ============================================================================
// MediaAudioPlayer Operations
// ============================================================================

/// Create a MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_create(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateMediaAudioPlayer { id: player_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Add source to MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_add_source(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
    #[smi] source_id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerAddSource {
        player_id,
        source_id,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Remove source from MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_remove_source(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
    #[smi] source_id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerRemoveSource {
        player_id,
        source_id,
    })
    .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Start MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_start(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerStart { player_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Stop MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_stop(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerStop { player_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
}

/// Destroy MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_destroy(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerDestroy { player_id })
        .map_err(|_| audio_err("Audio thread disconnected"))
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
#[op2(async(lazy))]
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

/// Normalize local audio source path variants.
///
/// - `wxfile://usr/...` -> `/user/...`
/// - `wxfile:///...` -> `/...`
#[inline]
fn normalize_local_src(src: &str) -> String {
    if let Some(rest) = src.strip_prefix("wxfile://") {
        if let Some(stripped) = rest.strip_prefix("usr/") {
            return format!("/user/{stripped}");
        }
        if let Some(stripped) = rest.strip_prefix("/usr/") {
            return format!("/user/{stripped}");
        }
        if rest.starts_with('/') {
            return rest.to_string();
        }
        return format!("/{rest}");
    }

    src.to_string()
}

/// Resolve a local path against code_dir for non-VFS paths.
#[inline]
fn resolve_path(code_dir: Option<&str>, path: &str) -> String {
    if path == "/code" {
        return code_dir.unwrap_or_default().to_string();
    }
    if let Some(stripped) = path.strip_prefix("/code/") {
        let mut full = PathBuf::from(code_dir.unwrap_or_default());
        full.push(stripped);
        return full.to_string_lossy().into_owned();
    }

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

#[inline]
fn resolve_local_src(
    code_dir: Option<&str>,
    vfs: Option<&VirtualFS>,
    src: &str,
) -> Result<String, AudioError> {
    let normalized = normalize_local_src(src);

    if let Some(vfs) = vfs {
        if !normalized.starts_with('/') {
            let vpath = format!("/code/{normalized}");
            return vfs
                .resolve(&vpath, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| {
                    audio_err(format!(
                        "Failed to resolve audio path '{}' (vpath='{}'): {}",
                        src, vpath, e
                    ))
                });
        }

        let is_virtual = normalized == "/code"
            || normalized.starts_with("/code/")
            || normalized == "/user"
            || normalized.starts_with("/user/")
            || normalized == "/cache"
            || normalized.starts_with("/cache/")
            || normalized == "/tmp"
            || normalized.starts_with("/tmp/");

        if is_virtual {
            return vfs
                .resolve(&normalized, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| {
                    audio_err(format!(
                        "Failed to resolve audio path '{}' (normalized='{}'): {}",
                        src, normalized, e
                    ))
                });
        }
    }

    Ok(resolve_path(code_dir, &normalized))
}

/// Load audio from URL or local path
/// - HTTP/HTTPS URLs: streaming download (edge-download-edge-play)
/// - Local paths: read file and load synchronously
#[op2(async(lazy), fast)]
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
        let (tx, code_dir, vfs) = {
            let st = state.borrow();
            let host = st.borrow::<HostOpState>();
            (
                host.audio_tx.clone(),
                host.code_dir.clone(),
                host.vfs.clone(),
            )
        };

        let full_path = resolve_local_src(code_dir.as_deref(), vfs.as_deref(), &src)?;
        tracing::debug!(
            "Loading local audio file: src={}, resolved={}",
            src,
            full_path
        );

        // Read file asynchronously
        let data = tokio::fs::read(&full_path).await.map_err(|e| {
            audio_err(format!(
                "Failed to read audio file (src='{}', resolved='{}'): {}",
                src, full_path, e
            ))
        })?;

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
#[op2(async(lazy), fast)]
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

#[cfg(test)]
mod tests {
    use super::{resolve_local_src, resolve_path};
    use shared::vfs::VirtualFS;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}", prefix, nanos))
    }

    #[test]
    fn resolve_path_maps_code_virtual_prefix() {
        let code_dir = make_temp_dir("migo_audio_code");
        fs::create_dir_all(code_dir.join("audio")).unwrap();

        let resolved = resolve_path(code_dir.to_str(), "/code/audio/bgm.mp3");
        let expected = code_dir.join("audio/bgm.mp3");

        assert_eq!(PathBuf::from(resolved), expected);

        let _ = fs::remove_dir_all(code_dir);
    }

    #[test]
    fn resolve_local_src_maps_user_virtual_path_with_vfs() {
        let base = make_temp_dir("migo_audio_vfs");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");

        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(user.join("gamecaches/audio")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&tmp).unwrap();

        let target = user.join("gamecaches/audio/bgm.mp3");
        fs::write(&target, b"audio-bytes").unwrap();

        let vfs = VirtualFS::new(code.clone(), user.clone(), cache, tmp);
        let resolved =
            resolve_local_src(code.to_str(), Some(&vfs), "/user/gamecaches/audio/bgm.mp3").unwrap();

        assert_eq!(PathBuf::from(resolved), target);

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn resolve_local_src_normalizes_wxfile_usr_prefix() {
        let base = make_temp_dir("migo_audio_wxfile");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");

        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(user.join("gamecaches/audio")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&tmp).unwrap();

        let target = user.join("gamecaches/audio/effect.mp3");
        fs::write(&target, b"audio-bytes").unwrap();

        let vfs = VirtualFS::new(code.clone(), user.clone(), cache, tmp);
        let resolved = resolve_local_src(
            code.to_str(),
            Some(&vfs),
            "wxfile://usr/gamecaches/audio/effect.mp3",
        )
        .unwrap();

        assert_eq!(PathBuf::from(resolved), target);

        let _ = fs::remove_dir_all(base);
    }
}
