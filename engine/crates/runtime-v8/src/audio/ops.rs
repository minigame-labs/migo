use deno_core::{JsBuffer, OpState, op2, v8};
use shared::{
    audio_resources::{
        AudioBufferFormat, AudioBufferKey, AudioResourceRegistry, PreparedAudioSnapshot,
    },
    error::{EngineError, ErrorCode},
    op_state::{AudioSender, HostOpState},
    protocol::audio_cmd::{
        AudioBufferId, AudioBufferInfo, AudioCmd, AudioContextId, AudioNodeId, InnerAudioId,
        InnerAudioInfo, InnerAudioState,
    },
    services::Scope,
    vfs::VirtualFS,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};
use tokio::{io::AsyncReadExt, sync::oneshot};

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

impl From<shared::audio_channel::AudioCommandSendError> for AudioError {
    fn from(error: shared::audio_channel::AudioCommandSendError) -> Self {
        match error {
            shared::audio_channel::AudioCommandSendError::Full(_) => AudioError::from(
                EngineError::from_detail(ErrorCode::InputSaturated, "audio command queue is full"),
            ),
            shared::audio_channel::AudioCommandSendError::ByteLimit(_) => {
                AudioError::from(EngineError::from_detail(
                    ErrorCode::InputSaturated,
                    "audio command queue byte limit exceeded",
                ))
            }
            shared::audio_channel::AudioCommandSendError::Disconnected(_) => AudioError::from(
                EngineError::from_detail(ErrorCode::Disconnected, "audio thread disconnected"),
            ),
        }
    }
}

impl From<shared::audio_channel::AudioCommandReserveError> for AudioError {
    fn from(error: shared::audio_channel::AudioCommandReserveError) -> Self {
        let detail = match error {
            shared::audio_channel::AudioCommandReserveError::Full => "audio command queue is full",
            shared::audio_channel::AudioCommandReserveError::ByteLimit => {
                "audio command queue byte limit exceeded"
            }
            shared::audio_channel::AudioCommandReserveError::Disconnected => {
                return AudioError::from(EngineError::from_detail(
                    ErrorCode::Disconnected,
                    "audio thread disconnected",
                ));
            }
        };
        AudioError::from(EngineError::from_detail(ErrorCode::InputSaturated, detail))
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

/// Maximum compressed/encoded bytes copied from one V8 audio input.
const MAX_ENCODED_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_WAVE_SHAPER_CURVE_BYTES: usize = 16 * 1024 * 1024;
/// Names and enum-like protocol strings never need to approach queue scale.
const MAX_AUDIO_CONTROL_STRING_BYTES: usize = 4 * 1024;
/// Frequency-response arrays are proportional to DSP work as well as queue use.
const MAX_FREQUENCY_RESPONSE_POINTS: usize = 16 * 1024;
const MAX_COPY_TO_CHANNEL_BYTES: usize = 64 * 1024 * 1024;
const LOCAL_AUDIO_READ_CHUNK_BYTES: usize = 8 * 1024;

fn validate_encoded_audio_size(len: usize) -> Result<(), EngineError> {
    if len > MAX_ENCODED_AUDIO_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("encoded audio input is {len} bytes; limit is {MAX_ENCODED_AUDIO_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn validate_scheduled_time(name: &str, value: f64) -> Result<f64, AudioError> {
    if !value.is_finite() || value < 0.0 {
        return Err(audio_err(format!(
            "{name} must be a finite, non-negative number"
        )));
    }
    Ok(value)
}

fn validate_optional_duration(value: f64) -> Result<Option<f64>, AudioError> {
    if value == -1.0 {
        return Ok(None);
    }
    validate_scheduled_time("duration", value).map(Some)
}

fn validate_wave_shaper_curve_size(len: usize) -> Result<(), EngineError> {
    if !len.is_multiple_of(std::mem::size_of::<f32>()) {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "WaveShaper curve bytes must be f32 aligned",
        ));
    }
    if len > MAX_WAVE_SHAPER_CURVE_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("WaveShaper curve is {len} bytes; limit is {MAX_WAVE_SHAPER_CURVE_BYTES}"),
        ));
    }
    Ok(())
}

fn validate_audio_control_string(field: &str, value: &str) -> Result<(), EngineError> {
    if value.len() > MAX_AUDIO_CONTROL_STRING_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!(
                "{field} is {} bytes; limit is {MAX_AUDIO_CONTROL_STRING_BYTES}",
                value.len()
            ),
        ));
    }
    Ok(())
}

fn validate_frequency_response_points(len: usize) -> Result<(), EngineError> {
    if len > MAX_FREQUENCY_RESPONSE_POINTS {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!(
                "frequency response has {len} points; limit is {MAX_FREQUENCY_RESPONSE_POINTS}"
            ),
        ));
    }
    Ok(())
}

fn validate_frequency_response_bytes(bytes: usize) -> Result<(), EngineError> {
    if !bytes.is_multiple_of(std::mem::size_of::<f32>()) {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "frequency response bytes must be f32 aligned",
        ));
    }
    validate_frequency_response_points(bytes / std::mem::size_of::<f32>())
}

fn validate_copy_to_channel_size(len: usize) -> Result<(), EngineError> {
    if !len.is_multiple_of(std::mem::size_of::<f32>()) {
        return Err(EngineError::from_detail(
            ErrorCode::InvalidArgument,
            "copyToChannel data bytes must be f32 aligned",
        ));
    }
    if len > MAX_COPY_TO_CHANNEL_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("copyToChannel data is {len} bytes; limit is {MAX_COPY_TO_CHANNEL_BYTES}"),
        ));
    }
    Ok(())
}

fn next_local_audio_capacity(
    current_capacity: usize,
    current_len: usize,
    incoming: usize,
) -> Result<Option<usize>, EngineError> {
    let needed = current_len.checked_add(incoming).ok_or_else(|| {
        EngineError::from_detail(
            ErrorCode::InputSaturated,
            "encoded audio input size overflow",
        )
    })?;
    if needed > MAX_ENCODED_AUDIO_BYTES {
        return Err(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("encoded audio input exceeds the {MAX_ENCODED_AUDIO_BYTES} byte limit"),
        ));
    }
    if needed <= current_capacity {
        return Ok(None);
    }

    // Metadata can become stale while a file grows. Grow geometrically from a
    // small first chunk so that this fallback stays amortized O(n), but never
    // request capacity beyond the input cap.
    let target = needed
        .max(LOCAL_AUDIO_READ_CHUNK_BYTES)
        .max(current_capacity.saturating_mul(2))
        .min(MAX_ENCODED_AUDIO_BYTES);
    Ok(Some(target))
}

async fn read_capped_local_audio(file: tokio::fs::File) -> Result<Vec<u8>, AudioError> {
    let file_len = usize::try_from(
        file.metadata()
            .await
            .map_err(|_| audio_err("Failed to read local audio file"))?
            .len(),
    )
    .unwrap_or(usize::MAX);
    read_capped_local_audio_with_capacity_hint(file, file_len).await
}

async fn read_capped_local_audio_with_capacity_hint(
    mut file: tokio::fs::File,
    file_len: usize,
) -> Result<Vec<u8>, AudioError> {
    if file_len > MAX_ENCODED_AUDIO_BYTES {
        return Err(AudioError::from(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("encoded audio input exceeds the {MAX_ENCODED_AUDIO_BYTES} byte limit"),
        )));
    }

    let mut data = Vec::new();
    data.try_reserve_exact(file_len)
        .map_err(|_| audio_err("Failed to allocate local audio buffer"))?;
    if data.capacity() > MAX_ENCODED_AUDIO_BYTES {
        return Err(AudioError::from(EngineError::from_detail(
            ErrorCode::InputSaturated,
            format!("encoded audio input exceeds the {MAX_ENCODED_AUDIO_BYTES} byte limit"),
        )));
    }
    let mut chunk = [0_u8; LOCAL_AUDIO_READ_CHUNK_BYTES];

    loop {
        let count = file
            .read(&mut chunk)
            .await
            .map_err(|_| audio_err("Failed to read local audio file"))?;
        if count == 0 {
            return Ok(data);
        }

        if let Some(target_capacity) = next_local_audio_capacity(data.capacity(), data.len(), count)
            .map_err(AudioError::from)?
        {
            data.try_reserve_exact(target_capacity - data.len())
                .map_err(|_| audio_err("Failed to allocate local audio buffer"))?;
            if data.capacity() > MAX_ENCODED_AUDIO_BYTES {
                return Err(AudioError::from(EngineError::from_detail(
                    ErrorCode::InputSaturated,
                    format!("encoded audio input exceeds the {MAX_ENCODED_AUDIO_BYTES} byte limit"),
                )));
            }
        }
        data.extend_from_slice(&chunk[..count]);
    }
}

// ============================================================================
// Context Operations
// ============================================================================

/// Create an AudioContext with a JS-allocated id (fire-and-forget).
///
/// The id is generated on the JS side so `new AudioContext()` is usable
/// synchronously (browser semantics) instead of racing an async round-trip —
/// otherwise `createGain()` on the next line runs with a null context id and
/// the smi decode fails with "expected i32". Ordering is safe: this command and
/// every later node op share one FIFO channel, so the context is created before
/// any node that references it.
#[op2(fast)]
pub fn op_audio_create_context(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] sample_rate: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    let sample_rate_opt = if sample_rate == 0 {
        None
    } else {
        Some(sample_rate)
    };

    tx.send(AudioCmd::CreateContext {
        ctx_id,
        sample_rate: sample_rate_opt,
    })
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
    .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Release an abandoned AudioContext from a `FinalizationRegistry` callback.
///
/// Unlike the explicit close operation this is fire-and-forget, idempotent,
/// and carries no response sender that could outlive the collected wrapper.
#[op2(fast)]
pub fn op_audio_release_context(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::ReleaseContext { ctx_id })
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
    .map_err(AudioError::from)?;

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
    .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// Buffer Operations
// ============================================================================

#[op2(async(lazy), fast)]
#[serde]
pub fn op_audio_decode_audio_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    data: v8::Local<v8::ArrayBuffer>,
) -> Result<
    impl std::future::Future<Output = Result<AudioBufferInfo, AudioError>> + use<>,
    AudioError,
> {
    if data.was_detached() || !data.is_detachable() {
        return Err(audio_err("audioData is detached or cannot be detached"));
    }
    let len = data.byte_length();
    validate_encoded_audio_size(len).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx.try_reserve_data(len).map_err(AudioError::from)?;

    let backing = data.get_backing_store();
    if backing.is_shared() {
        return Err(audio_err("audioData must not be shared"));
    }
    if backing.byte_length() < len {
        return Err(audio_err(
            "audioData backing is shorter than its visible length",
        ));
    }
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(len)
        .map_err(|_| audio_err("encoded audio input allocation failed"))?;
    if len != 0 {
        let ptr = backing
            .data()
            .ok_or_else(|| audio_err("audioData backing has no data"))?;
        // JavaScript cannot resize or detach this buffer while the synchronous
        // op entry point is executing. The backing-store handle keeps the
        // allocation alive until the copy completes.
        let source = unsafe { std::slice::from_raw_parts(ptr.as_ptr().cast::<u8>(), len) };
        owned.extend_from_slice(source);
    }
    drop(backing);
    if data.detach(None) != Some(true) {
        return Err(audio_err("audioData could not be detached"));
    }

    let (resp_tx, resp_rx) = oneshot::channel();
    tx.send_reserved(
        AudioCmd::DecodeAudioData {
            ctx_id,
            data: std::sync::Arc::new(owned),
            resp: resp_tx,
        },
        permit,
    )
    .map_err(AudioError::from)?;

    Ok(async move {
        resp_rx
            .await
            .map_err(|_| audio_err("Response channel closed"))?
            .map_err(AudioError::from)
    })
}

fn audio_buffer_key(state: &Rc<RefCell<OpState>>, serial: AudioBufferId) -> AudioBufferKey {
    let state = state.borrow();
    AudioBufferKey {
        runtime_generation: state.borrow::<HostOpState>().runtime_generation,
        serial,
    }
}

fn prepare_snapshot_from_backing(
    resources: &AudioResourceRegistry,
    key: AudioBufferKey,
    backing: v8::Local<v8::ArrayBuffer>,
    expected_bytes: usize,
) -> Result<PreparedAudioSnapshot, AudioError> {
    if backing.was_detached() || !backing.is_detachable() || backing.byte_length() != expected_bytes
    {
        return Err(audio_err(
            "AudioBuffer backing is detached, non-detachable, or has an invalid length",
        ));
    }
    let store = backing.get_backing_store();
    if store.is_shared() || store.is_resizable_by_user_javascript() {
        return Err(audio_err(
            "AudioBuffer backing must be non-shared and fixed-length",
        ));
    }
    let sample_count = expected_bytes
        .checked_div(std::mem::size_of::<f32>())
        .ok_or_else(|| audio_err("AudioBuffer sample count overflow"))?;
    let data = store
        .data()
        .ok_or_else(|| audio_err("AudioBuffer backing has no data"))?;
    if (data.as_ptr() as usize) % std::mem::align_of::<f32>() != 0 {
        return Err(audio_err("AudioBuffer backing is not f32-aligned"));
    }

    // This is a non-shared, fixed-length ArrayBuffer and JavaScript cannot run
    // concurrently while this synchronous op is executing. `store` keeps the
    // allocation alive for the full borrow; it is dropped before publication.
    let planar = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<f32>(), sample_count) };
    let prepared = resources.prepare_snapshot(key, Some(planar))?;
    drop(store);
    Ok(prepared)
}

#[op2]
pub fn op_audio_reserve_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] channels: u32,
    #[smi] length: u32,
    #[smi] sample_rate: u32,
) -> Result<AudioBufferId, AudioError> {
    let state_ref = state.borrow();
    let host = state_ref.borrow::<HostOpState>();
    let lease = host.audio_tx.resources()?.reserve_backing(
        host.runtime_generation,
        AudioBufferFormat {
            channels,
            frames: length,
            sample_rate,
        },
    )?;
    Ok(lease.key().serial)
}

fn release_global_audio_buffer(state: &Rc<RefCell<OpState>>, buffer_id: AudioBufferId) {
    let key = audio_buffer_key(state, buffer_id);
    let state = state.borrow();
    if let Ok(resources) = state.borrow::<HostOpState>().audio_tx.resources() {
        resources.release_buffer(key);
    }
}

#[op2(fast)]
pub fn op_audio_abort_buffer(state: Rc<RefCell<OpState>>, #[smi] buffer_id: AudioBufferId) {
    release_global_audio_buffer(&state, buffer_id);
}

/// Release a global AudioBuffer backing from its finalizer. Missing/stale keys
/// are intentionally idempotent no-ops.
#[op2(fast)]
pub fn op_audio_release_buffer(state: Rc<RefCell<OpState>>, #[smi] buffer_id: AudioBufferId) {
    release_global_audio_buffer(&state, buffer_id);
}

#[op2]
#[arraybuffer]
pub fn op_audio_materialize_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] buffer_id: AudioBufferId,
) -> Result<Vec<f32>, AudioError> {
    let key = audio_buffer_key(&state, buffer_id);
    let state = state.borrow();
    let prepared = state
        .borrow::<HostOpState>()
        .audio_tx
        .resources()?
        .prepare_materialize(key)?;
    Ok(prepared.commit())
}

fn publish_snapshot(
    state: Rc<RefCell<OpState>>,
    ctx_id: AudioContextId,
    node_id: AudioNodeId,
    buffer_id: AudioBufferId,
    backing: Option<v8::Local<v8::ArrayBuffer>>,
    when: Option<(f64, f64, f64)>,
) -> Result<(), AudioError> {
    let timing = match when {
        Some((when, offset, duration)) => Some((
            validate_scheduled_time("when", when)?,
            validate_scheduled_time("offset", offset)?,
            validate_optional_duration(duration)?,
        )),
        None => None,
    };
    let key = (buffer_id != 0).then(|| audio_buffer_key(&state, buffer_id));
    if key.is_none() && backing.is_some() {
        return Err(audio_err(
            "a null AudioBuffer id cannot carry a backing store",
        ));
    }
    let tx = {
        let state_ref = state.borrow();
        let host = state_ref.borrow::<HostOpState>();
        host.audio_tx.clone()
    };
    // Claim the queue slot before copying as much as 64 MiB of PCM. A failed
    // preparation drops this permit, while a later restart fence prevents the
    // detach/registry commit closure from running.
    let permit = tx.try_reserve_data(0).map_err(AudioError::from)?;
    let prepared = {
        let state_ref = state.borrow();
        let host = state_ref.borrow::<HostOpState>();
        let resources = host.audio_tx.resources()?;
        match key {
            None => None,
            Some(key) => {
                let prepared = match backing {
                    Some(buffer) => {
                        let format = resources
                            .format(key)
                            .ok_or_else(|| audio_err("unknown AudioBuffer id"))?;
                        let expected_bytes = usize::try_from(format.channels)
                            .ok()
                            .and_then(|channels| {
                                usize::try_from(format.frames)
                                    .ok()
                                    .and_then(|frames| channels.checked_mul(frames))
                            })
                            .and_then(|samples| samples.checked_mul(std::mem::size_of::<f32>()))
                            .ok_or_else(|| audio_err("AudioBuffer backing size overflow"))?;
                        prepare_snapshot_from_backing(resources, key, buffer, expected_bytes)?
                    }
                    None => resources.prepare_snapshot(key, None)?,
                };
                Some(prepared)
            }
        }
    };
    let snapshot = prepared.as_ref().map(|prepared| prepared.snapshot());
    let command = match timing {
        Some((when, offset, duration)) => AudioCmd::StartBuffer {
            ctx_id,
            node_id,
            buffer: snapshot,
            when,
            offset,
            duration,
        },
        None => AudioCmd::SetStartedBuffer {
            ctx_id,
            node_id,
            buffer: snapshot,
        },
    };
    tx.send_reserved_committing(command, permit, move || {
        if let Some(backing) = backing {
            if backing.detach(None) != Some(true) {
                // The buffer was validated while JavaScript was paused, so
                // this indicates a V8 invariant failure. Never unwind through
                // the op/JNI boundary after the audio command was accepted.
                tracing::error!("validated AudioBuffer backing failed to detach");
            }
        }
        if let Some(prepared) = prepared {
            let _ = prepared.commit();
        }
    })
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_start_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[smi] buffer_id: AudioBufferId,
    backing: Option<v8::Local<v8::ArrayBuffer>>,
    when: f64,
    offset: f64,
    duration: f64,
) -> Result<(), AudioError> {
    publish_snapshot(
        state,
        ctx_id,
        node_id,
        buffer_id,
        backing,
        Some((when, offset, duration)),
    )
}

#[op2(fast)]
pub fn op_audio_set_started_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[smi] buffer_id: AudioBufferId,
    backing: Option<v8::Local<v8::ArrayBuffer>>,
) -> Result<(), AudioError> {
    publish_snapshot(state, ctx_id, node_id, buffer_id, backing, None)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_buffer(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] node_id: AudioNodeId,
    #[smi] buffer_id: AudioBufferId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::SetBuffer {
        ctx_id,
        node_id,
        buffer_id: (buffer_id != 0).then_some(buffer_id),
    })
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
    let when = validate_scheduled_time("when", when)?;
    let offset = validate_scheduled_time("offset", offset)?;
    let duration_opt = validate_optional_duration(duration)?;
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::Start {
        node_id,
        when,
        offset,
        duration: duration_opt,
        resp: resp_tx,
    })
    .map_err(AudioError::from)?;

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
    let when = validate_scheduled_time("when", when)?;
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::Stop {
        node_id,
        when,
        resp: resp_tx,
    })
    .map_err(AudioError::from)?;

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
    if !loop_start.is_finite() || !loop_end.is_finite() {
        return Err(audio_err("loopStart and loopEnd must be finite numbers"));
    }
    let tx = get_audio_tx(state);

    tx.send(AudioCmd::SetLoop {
        node_id,
        loop_enabled,
        loop_start,
        loop_end,
    })
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

/// Set an AudioParam's current value now by node + param name (fire and forget).
#[op2(fast)]
pub fn op_audio_set_node_param(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: &str,
    value: f32,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetNodeParam {
            node_id,
            param_name: param_name.to_owned(),
            value,
        },
        permit,
    )
    .map_err(AudioError::from)
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
    .map_err(AudioError::from)?;

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
    .map_err(AudioError::from)?;

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
    #[string] param_name: &str,
    value: f32,
    time: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::AudioParamSetValueAtTime {
            node_id,
            param_name: param_name.to_owned(),
            value,
            time,
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_param_linear_ramp(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: &str,
    value: f32,
    end_time: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::AudioParamLinearRamp {
            node_id,
            param_name: param_name.to_owned(),
            value,
            end_time,
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_param_exponential_ramp(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: &str,
    value: f32,
    end_time: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::AudioParamExponentialRamp {
            node_id,
            param_name: param_name.to_owned(),
            value,
            end_time,
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_param_set_target(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: &str,
    target: f32,
    start_time: f64,
    time_constant: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::AudioParamSetTarget {
            node_id,
            param_name: param_name.to_owned(),
            target,
            start_time,
            time_constant,
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_param_cancel_scheduled(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] param_name: &str,
    cancel_time: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("AudioParam name", param_name).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(param_name.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::AudioParamCancelScheduled {
            node_id,
            param_name: param_name.to_owned(),
            cancel_time,
        },
        permit,
    )
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_oscillator_type(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] osc_type: &str,
) -> Result<(), AudioError> {
    validate_audio_control_string("oscillator type", osc_type).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(osc_type.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetOscillatorType {
            node_id,
            osc_type: osc_type.to_owned(),
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_start_oscillator(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let when = validate_scheduled_time("when", when)?;
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StartOscillator { node_id, when })
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_stop_oscillator(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let when = validate_scheduled_time("when", when)?;
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StopOscillator { node_id, when })
        .map_err(AudioError::from)
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
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_biquad_filter_type(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] filter_type: &str,
) -> Result<(), AudioError> {
    validate_audio_control_string("biquad filter type", filter_type).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(filter_type.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetBiquadFilterType {
            node_id,
            filter_type: filter_type.to_owned(),
        },
        permit,
    )
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2]
pub fn op_audio_set_wave_shaper_curve(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[buffer] curve_bytes: Option<JsBuffer>,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    let byte_len = curve_bytes.as_ref().map_or(0, |buf| buf.len());
    validate_wave_shaper_curve_size(byte_len).map_err(AudioError::from)?;
    let permit = tx.try_reserve_data(byte_len).map_err(AudioError::from)?;
    let curve = curve_bytes.map(|buf| {
        buf.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect::<Vec<f32>>()
    });
    tx.send_reserved(AudioCmd::SetWaveShaperCurve { node_id, curve }, permit)
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_wave_shaper_oversample(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] oversample: &str,
) -> Result<(), AudioError> {
    validate_audio_control_string("WaveShaper oversample", oversample).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(oversample.len())
        .map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetWaveShaperOversample {
            node_id,
            oversample: oversample.to_owned(),
        },
        permit,
    )
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_analyser_fft_size(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[smi] fft_size: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::SetAnalyserFftSize { node_id, fft_size })
        .map_err(AudioError::from)
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
    .map_err(AudioError::from)?;
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
    .map_err(AudioError::from)?;
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_panning_model(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] model: &str,
) -> Result<(), AudioError> {
    validate_audio_control_string("panning model", model).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx.try_reserve_data(model.len()).map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetPanningModel {
            node_id,
            model: model.to_owned(),
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_distance_model(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] model: &str,
) -> Result<(), AudioError> {
    validate_audio_control_string("distance model", model).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx.try_reserve_data(model.len()).map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetDistanceModel {
            node_id,
            model: model.to_owned(),
        },
        permit,
    )
    .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_set_panner_scalar(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[string] prop: &str,
    value: f64,
) -> Result<(), AudioError> {
    validate_audio_control_string("panner property", prop).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx.try_reserve_data(prop.len()).map_err(AudioError::from)?;
    tx.send_reserved(
        AudioCmd::SetPannerScalar {
            node_id,
            prop: prop.to_owned(),
            value,
        },
        permit,
    )
    .map_err(AudioError::from)
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
    .map_err(AudioError::from)
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
    .map_err(AudioError::from)
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
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_start_constant_source(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let when = validate_scheduled_time("when", when)?;
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StartConstantSource { node_id, when })
        .map_err(AudioError::from)
}

#[op2(fast)]
pub fn op_audio_stop_constant_source(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    when: f64,
) -> Result<(), AudioError> {
    let when = validate_scheduled_time("when", when)?;
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::StopConstantSource { node_id, when })
        .map_err(AudioError::from)
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
    // WebAudio: coefficient arrays must be non-empty, <= 20 long, feedback[0] != 0,
    // and finite. Reject here so the audio thread never divides by an empty length.
    if feedforward.is_empty() || feedback.is_empty() {
        return Err(audio_err(
            "createIIRFilter: feedforward and feedback must be non-empty",
        ));
    }
    if feedforward.len() > 20 || feedback.len() > 20 {
        return Err(audio_err(
            "createIIRFilter: coefficient arrays must have at most 20 elements",
        ));
    }
    if feedback[0] == 0.0 {
        return Err(audio_err("createIIRFilter: feedback[0] must not be zero"));
    }
    if feedforward
        .iter()
        .chain(feedback.iter())
        .any(|v| !v.is_finite())
    {
        return Err(audio_err("createIIRFilter: coefficients must be finite"));
    }

    let tx = get_audio_tx(state);
    tx.send(AudioCmd::CreateIIRFilter {
        ctx_id,
        node_id,
        feedforward,
        feedback,
    })
    .map_err(AudioError::from)
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
    .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Get channel data from a buffer (returns raw f32 bytes)
#[op2(async(lazy), fast)]
#[arraybuffer]
pub async fn op_audio_get_channel_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] buffer_id: AudioBufferId,
    #[smi] channel: u32,
) -> Result<Vec<f32>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::GetChannelData {
        ctx_id,
        buffer_id,
        channel,
        resp: resp_tx,
    })
    .map_err(AudioError::from)?;

    let data = resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)?;

    Ok(data)
}

/// Get all decoded channels as one exact planar `ArrayBuffer`.
#[op2(async(lazy), fast)]
#[arraybuffer]
pub async fn op_audio_take_decoded_buffer_data(
    state: Rc<RefCell<OpState>>,
    #[smi] ctx_id: AudioContextId,
    #[smi] buffer_id: AudioBufferId,
) -> Result<Vec<f32>, AudioError> {
    let tx = get_audio_tx(state);
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send(AudioCmd::TakeDecodedBufferData {
        ctx_id,
        buffer_id,
        resp: resp_tx,
    })
    .map_err(AudioError::from)?;

    let channels = resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)?;

    Ok(channels)
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
    validate_copy_to_channel_size(data_bytes.len()).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(data_bytes.len())
        .map_err(AudioError::from)?;
    let (resp_tx, resp_rx) = oneshot::channel();

    // Convert raw bytes to Vec<f32>
    let data: Vec<f32> = data_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    tx.send_reserved(
        AudioCmd::CopyToChannel {
            ctx_id,
            buffer_id,
            data,
            channel,
            start,
            resp: resp_tx,
        },
        permit,
    )
    .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

// ============================================================================
// Frequency Response & Analysis
// ============================================================================

/// Get frequency response from BiquadFilterNode or IIRFilterNode.
/// Returns the magnitude and phase response vectors.
#[op2(async(lazy))]
#[serde]
pub async fn op_audio_get_frequency_response(
    state: Rc<RefCell<OpState>>,
    #[smi] node_id: AudioNodeId,
    #[buffer] frequencies: JsBuffer,
) -> Result<(Vec<f32>, Vec<f32>), AudioError> {
    validate_frequency_response_bytes(frequencies.len()).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx
        .try_reserve_data(frequencies.len())
        .map_err(AudioError::from)?;
    let frequencies = frequencies
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send_reserved(
        AudioCmd::GetFrequencyResponse {
            node_id,
            frequencies,
            resp: resp_tx,
        },
        permit,
    )
    .map_err(AudioError::from)?;

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
    .map_err(AudioError::from)?;

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
    .map_err(AudioError::from)?;

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
    .map_err(AudioError::from)?;

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
    crate::permission::require_scope(state, Scope::Record)
        .map_err(|denied| audio_err(denied.to_string()))?;
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
    crate::permission::require_scope(state, Scope::Record)
        .map_err(|denied| audio_err(denied.to_string()))?;
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
    crate::permission::require_scope(state, Scope::Record)
        .map_err(|denied| audio_err(denied.to_string()))?;
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
        .map_err(AudioError::from)
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
    .map_err(AudioError::from)
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
    .map_err(AudioError::from)
}

/// Start MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_start(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerStart { player_id })
        .map_err(AudioError::from)
}

/// Stop MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_stop(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerStop { player_id })
        .map_err(AudioError::from)
}

/// Destroy MediaAudioPlayer (fire and forget)
#[op2(fast)]
pub fn op_media_audio_player_destroy(
    state: Rc<RefCell<OpState>>,
    #[smi] player_id: u32,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::MediaAudioPlayerDestroy { player_id })
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
        .map_err(AudioError::from)
}

/// Destroy an InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_destroy(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::DestroyInnerAudio { id })
        .map_err(AudioError::from)
}

/// Load audio data into InnerAudioContext (full load mode - deprecated, use op_inner_audio_load_url)
#[op2(async(lazy))]
#[serde]
pub async fn op_inner_audio_load(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    #[buffer] data: JsBuffer,
) -> Result<InnerAudioInfo, AudioError> {
    validate_encoded_audio_size(data.len()).map_err(AudioError::from)?;
    let tx = get_audio_tx(state);
    let permit = tx.try_reserve_data(data.len()).map_err(AudioError::from)?;
    let (resp_tx, resp_rx) = oneshot::channel();

    tx.send_reserved(
        AudioCmd::InnerAudioLoad {
            id,
            data: data.to_vec(),
            resp: resp_tx,
        },
        permit,
    )
    .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

/// Parse a remote audio URL without reclassifying ordinary local/VFS paths.
/// URL schemes are case-insensitive; malformed strings that explicitly claim
/// HTTP(S) are errors rather than filesystem fallbacks.
fn parse_remote_audio_url(src: &str) -> Result<Option<deno_core::url::Url>, AudioError> {
    match deno_core::url::Url::parse(src) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(Some(url)),
        Ok(_) => Ok(None),
        Err(error) => {
            let is_http = src
                .get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"));
            let is_https = src
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"));
            if is_http || is_https {
                Err(audio_err(format!("Invalid audio URL: {error}")))
            } else {
                Ok(None)
            }
        }
    }
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

#[derive(Debug, PartialEq, Eq)]
enum LocalAudioSource {
    /// A virtual path that must be opened by the VFS itself. Keeping it virtual
    /// prevents callers from accidentally reintroducing resolve-then-open.
    Sandboxed { virtual_path: String },
    /// Tooling/headless mode has no sandbox contract and retains the legacy
    /// code-directory-relative behavior.
    Unconfined { path: String },
}

#[inline]
fn resolve_local_src(
    code_dir: Option<&str>,
    vfs: Option<&VirtualFS>,
    src: &str,
) -> Result<LocalAudioSource, AudioError> {
    if let Some(vfs) = vfs {
        if !src.starts_with('/') {
            return Ok(LocalAudioSource::Sandboxed {
                virtual_path: format!("/code/{src}"),
            });
        }

        if vfs.is_virtual_path(src) {
            return Ok(LocalAudioSource::Sandboxed {
                virtual_path: src.to_owned(),
            });
        }

        // Never include the rejected input or a host path in a JS-visible
        // error. The caller only needs to know that policy denied it.
        return Err(audio_err("Local audio path is not permitted"));
    }

    // No VFS (headless / tooling): fall back to code_dir-relative resolution.
    Ok(LocalAudioSource::Unconfined {
        path: resolve_path(code_dir, src),
    })
}

async fn open_local_audio_source(
    vfs: Option<Arc<VirtualFS>>,
    source: LocalAudioSource,
) -> Result<tokio::fs::File, AudioError> {
    match source {
        LocalAudioSource::Sandboxed { virtual_path } => {
            let vfs = vfs.ok_or_else(|| audio_err("Failed to open local audio file"))?;
            let file =
                tokio::task::spawn_blocking(move || vfs.open_regular_for_read(&virtual_path))
                    .await
                    .map_err(|_| audio_err("Failed to open local audio file"))?
                    .map_err(|_| audio_err("Failed to open local audio file"))?;
            Ok(tokio::fs::File::from_std(file))
        }
        LocalAudioSource::Unconfined { path } => tokio::fs::File::open(path)
            .await
            .map_err(|_| audio_err("Failed to open local audio file")),
    }
}

/// Load audio from URL or local path
/// - HTTP/HTTPS URLs: streaming download (edge-download-edge-play)
/// - Local paths: read file and load synchronously
#[op2(async(lazy), fast)]
pub fn op_inner_audio_load_url(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
    #[string] src: &str,
) -> Result<impl std::future::Future<Output = Result<(), AudioError>> + use<>, AudioError> {
    let src = prepare_inner_audio_src(src)?;
    Ok(op_inner_audio_load_url_owned(state, id, src))
}

fn prepare_inner_audio_src(src: &str) -> Result<String, AudioError> {
    validate_audio_control_string("audio source URL", src).map_err(AudioError::from)?;
    Ok(src.to_owned())
}

async fn op_inner_audio_load_url_owned(
    state: Rc<RefCell<OpState>>,
    id: InnerAudioId,
    src: String,
) -> Result<(), AudioError> {
    if let Some(url) = parse_remote_audio_url(&src)? {
        // HTTP URL - use streaming download
        {
            let st = state.borrow();
            crate::network::gate::enforce_from_state(
                &url,
                &st,
                crate::network::gate::GateKind::AudioStream,
            )
            .map_err(|error| audio_err(error.to_string()))?;
        }
        let tx = get_audio_tx(state);
        let (resp_tx, resp_rx) = oneshot::channel();

        tx.send(AudioCmd::InnerAudioLoadUrl {
            id,
            url: src,
            resp: resp_tx,
        })
        .map_err(AudioError::from)?;

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

        let source = resolve_local_src(code_dir.as_deref(), vfs.as_deref(), &src)?;
        // Admit the operation before it reaches the blocking filesystem pool.
        // This bounds concurrent opens as well as the later allocation; every
        // failure releases the RAII permit, and successful tiny loads shrink it
        // to the Vec capacity retained by the queued command.
        let mut permit = tx
            .try_reserve_data(MAX_ENCODED_AUDIO_BYTES)
            .map_err(AudioError::from)?;
        let file = open_local_audio_source(vfs, source).await?;
        let data = read_capped_local_audio(file).await?;
        if data.capacity() > MAX_ENCODED_AUDIO_BYTES {
            return Err(AudioError::from(EngineError::from_detail(
                ErrorCode::InputSaturated,
                "local audio buffer capacity exceeds its reservation",
            )));
        }
        // The file reader uses metadata only as an allocation hint, so charge
        // the actual retained Vec capacity before publishing its command.
        permit.shrink_to(data.capacity());

        let (resp_tx, resp_rx) = oneshot::channel();

        tx.send_reserved(
            AudioCmd::InnerAudioLoad {
                id,
                data,
                resp: resp_tx,
            },
            permit,
        )
        .map_err(AudioError::from)?;

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
        .map_err(AudioError::from)
}

/// Pause InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_pause(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioPause { id })
        .map_err(AudioError::from)
}

/// Stop InnerAudioContext (fire and forget)
#[op2(fast)]
pub fn op_inner_audio_stop(
    state: Rc<RefCell<OpState>>,
    #[smi] id: InnerAudioId,
) -> Result<(), AudioError> {
    let tx = get_audio_tx(state);
    tx.send(AudioCmd::InnerAudioStop { id })
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)
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
        .map_err(AudioError::from)?;

    resp_rx
        .await
        .map_err(|_| audio_err("Response channel closed"))?
        .map_err(AudioError::from)
}

#[cfg(test)]
mod tests {
    use super::{
        AudioCmd, AudioError, AudioSender, LocalAudioSource, MAX_AUDIO_CONTROL_STRING_BYTES,
        MAX_COPY_TO_CHANNEL_BYTES, MAX_ENCODED_AUDIO_BYTES, MAX_FREQUENCY_RESPONSE_POINTS,
        MAX_WAVE_SHAPER_CURVE_BYTES, next_local_audio_capacity, open_local_audio_source,
        parse_remote_audio_url, prepare_inner_audio_src, read_capped_local_audio,
        read_capped_local_audio_with_capacity_hint, resolve_local_src, resolve_path,
        validate_audio_control_string, validate_copy_to_channel_size, validate_encoded_audio_size,
        validate_frequency_response_bytes, validate_frequency_response_points,
        validate_optional_duration, validate_scheduled_time, validate_wave_shaper_curve_size,
    };
    use shared::vfs::VirtualFS;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn audio_queue_full_is_not_reported_as_disconnected() {
        let (tx, _rx) = shared::audio_channel::channel();
        for ctx_id in 0..shared::audio_channel::AUDIO_COMMAND_CAPACITY as u32 {
            tx.try_send(AudioCmd::CreateContext {
                ctx_id,
                sample_rate: None,
            })
            .expect("fixture fills the data queue");
        }
        let sender = AudioSender::new(tx, shared::channel::ThreadWakeup::new());
        let (resp, mut response) = tokio::sync::oneshot::channel();

        let error = sender
            .send(AudioCmd::CloseContext { ctx_id: 9, resp })
            .map_err(AudioError::from)
            .expect_err("limit + 1 must fail immediately");

        assert!(error.to_string().contains("InputSaturated"));
        assert!(error.to_string().contains("audio command queue is full"));
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }

    #[test]
    fn v8_encoded_audio_is_capped_before_copying() {
        let source = include_str!("ops.rs");
        for (name, end_marker, copy_marker) in [
            (
                "pub fn op_audio_decode_audio_data",
                "// ============================================================================\n// BufferSourceNode Operations",
                "extend_from_slice",
            ),
            (
                "pub async fn op_inner_audio_load",
                "/// Parse a remote audio URL",
                ".to_vec()",
            ),
        ] {
            let start = source.find(name).expect("audio input op");
            let end = source[start..]
                .find(end_marker)
                .map(|offset| start + offset)
                .expect("end of audio input op");
            let body = &source[start..end];
            let cap = body
                .find("validate_encoded_audio_size")
                .expect("encoded-size cap must be checked in the op");
            let copy = body
                .find(copy_marker)
                .expect("the V8 backing store is copied by this op");
            assert!(cap < copy, "{name} checked the cap after allocating");
        }
    }

    #[test]
    fn decode_audio_data_detaches_the_exact_input_before_returning_the_future() {
        let rust = include_str!("ops.rs");
        let start = rust
            .find("pub fn op_audio_decode_audio_data")
            .expect("decode op entry point");
        let end = rust[start..]
            .find("fn audio_buffer_key")
            .map(|offset| start + offset)
            .expect("end of decode op");
        let body = &rust[start..end];

        assert!(body.contains("v8::Local<v8::ArrayBuffer>"));
        assert!(!body.contains("#[buffer] data: JsBuffer"));
        let reserve = body.find("try_reserve_data").expect("queue reservation");
        let copy = body.find("extend_from_slice").expect("bounded input copy");
        let detach = body.find("detach(None)").expect("synchronous detach");
        let future = body
            .find("async move")
            .expect("response future must start after ownership transfer");
        assert!(reserve < copy && copy < detach && detach < future);

        let js = include_str!("01_audio_context.js");
        let start = js.find("async decodeAudioData").expect("decode method");
        let end = js[start..]
            .find("  createBuffer(")
            .map(|offset| start + offset)
            .expect("end of decode method");
        let body = &js[start..end];
        assert!(
            body.contains(
                "op_audio_decode_audio_data(\n        this.#nativeId,\n        audioData"
            )
        );
        assert!(!body.contains("new Uint8Array(audioData)\n      )"));
        assert!(body.contains("new DOMException") && body.contains("\"DataCloneError\""));
        assert!(body.contains("const decodePromise"));
        assert!(body.contains("decodePromise.then"));
        assert!(
            !body.contains("if (errorCallback) {\n        errorCallback(error);\n        return;")
        );
    }

    #[test]
    fn scheduled_source_time_validation_is_uniform_and_native_defended() {
        for source in [
            include_str!("00_buffer_source_node.js"),
            include_str!("00_constant_source_node.js"),
            include_str!("00_oscillator_node.js"),
        ] {
            assert!(source.contains("InvalidStateError"));
            assert!(source.contains("validateScheduledTime"));
        }
        let node = include_str!("00_audio_node.js");
        assert!(node.contains("Number.isFinite"));
        assert!(node.contains("RangeError"));

        assert!(validate_scheduled_time("when", 0.0).is_ok());
        assert!(validate_scheduled_time("when", 1.25).is_ok());
        for invalid in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(validate_scheduled_time("when", invalid).is_err());
        }
        assert_eq!(validate_optional_duration(-1.0).unwrap(), None);
        assert_eq!(validate_optional_duration(0.0).unwrap(), Some(0.0));
        assert!(validate_optional_duration(-2.0).is_err());
        assert!(validate_optional_duration(f64::NAN).is_err());
    }

    #[test]
    fn encoded_audio_size_boundary_is_inclusive() {
        assert!(validate_encoded_audio_size(MAX_ENCODED_AUDIO_BYTES).is_ok());
        let error = validate_encoded_audio_size(MAX_ENCODED_AUDIO_BYTES + 1)
            .expect_err("limit + 1 must be rejected before copying");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
    }

    #[test]
    fn wave_shaper_curve_requires_f32_alignment_and_inclusive_byte_cap() {
        assert!(validate_wave_shaper_curve_size(0).is_ok());
        assert!(validate_wave_shaper_curve_size(MAX_WAVE_SHAPER_CURVE_BYTES).is_ok());

        let unaligned = validate_wave_shaper_curve_size(3)
            .expect_err("a partial f32 must not be silently truncated");
        assert_eq!(unaligned.code, shared::error::ErrorCode::InvalidArgument);

        let over_limit = validate_wave_shaper_curve_size(MAX_WAVE_SHAPER_CURVE_BYTES + 4)
            .expect_err("limit + one f32 must be rejected");
        assert_eq!(over_limit.code, shared::error::ErrorCode::InputSaturated);
    }

    #[test]
    fn every_content_controlled_audio_payload_has_a_single_item_boundary() {
        assert!(
            validate_audio_control_string("field", &"x".repeat(MAX_AUDIO_CONTROL_STRING_BYTES))
                .is_ok()
        );
        assert_eq!(
            validate_audio_control_string("field", &"x".repeat(MAX_AUDIO_CONTROL_STRING_BYTES + 1))
                .unwrap_err()
                .code,
            shared::error::ErrorCode::InputSaturated
        );
        assert!(validate_frequency_response_points(MAX_FREQUENCY_RESPONSE_POINTS).is_ok());
        assert_eq!(
            validate_frequency_response_points(MAX_FREQUENCY_RESPONSE_POINTS + 1)
                .unwrap_err()
                .code,
            shared::error::ErrorCode::InputSaturated
        );
        assert!(validate_copy_to_channel_size(MAX_COPY_TO_CHANNEL_BYTES).is_ok());
        assert_eq!(
            validate_copy_to_channel_size(MAX_COPY_TO_CHANNEL_BYTES + 4)
                .unwrap_err()
                .code,
            shared::error::ErrorCode::InputSaturated
        );
        assert_eq!(
            validate_copy_to_channel_size(3).unwrap_err().code,
            shared::error::ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn wave_shaper_reserves_before_converting_the_borrowed_js_buffer() {
        let source = include_str!("ops.rs");
        let start = source
            .find("pub fn op_audio_set_wave_shaper_curve")
            .expect("WaveShaper op");
        let end = source[start..]
            .find("#[op2(fast)]")
            .map(|offset| start + offset)
            .expect("following WaveShaper op");
        let body = &source[start..end];

        let borrowed = body
            .find("#[buffer] curve_bytes: Option<JsBuffer>")
            .unwrap();
        let reserve = body.find("try_reserve_data(byte_len)").unwrap();
        let convert = body.find("chunks_exact(4)").unwrap();
        let send = body.find("send_reserved").unwrap();
        assert!(borrowed < reserve && reserve < convert && convert < send);
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}", prefix, nanos))
    }

    #[test]
    fn capped_local_audio_reader_accepts_exact_limit_and_rejects_limit_plus_one() {
        let dir = make_temp_dir("migo_audio_capped_reader");
        fs::create_dir_all(&dir).unwrap();
        let exact = dir.join("exact.mp3");
        let over = dir.join("over.mp3");
        fs::write(&exact, vec![0_u8; MAX_ENCODED_AUDIO_BYTES]).unwrap();
        fs::write(&over, vec![0_u8; MAX_ENCODED_AUDIO_BYTES + 1]).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();

        let exact_data = runtime.block_on(async {
            read_capped_local_audio(tokio::fs::File::open(&exact).await.unwrap()).await
        });
        assert_eq!(exact_data.unwrap().len(), MAX_ENCODED_AUDIO_BYTES);
        let over_limit = runtime
            .block_on(async {
                read_capped_local_audio(tokio::fs::File::open(&over).await.unwrap()).await
            })
            .expect_err("limit + 1 local audio input must be rejected");
        assert!(over_limit.to_string().contains("InputSaturated"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn capped_local_audio_reader_uses_open_file_metadata_as_the_initial_capacity_hint() {
        let dir = make_temp_dir("migo_audio_small_reader");
        fs::create_dir_all(&dir).unwrap();
        let tiny = dir.join("tiny.mp3");
        fs::write(&tiny, b"tiny").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let data = runtime.block_on(async {
            read_capped_local_audio(tokio::fs::File::open(&tiny).await.unwrap())
                .await
                .unwrap()
        });

        assert!(data.capacity() < MAX_ENCODED_AUDIO_BYTES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn capped_local_audio_reader_handles_metadata_underestimation_without_overcharging() {
        let dir = make_temp_dir("migo_audio_growing_reader");
        fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("grown.mp3");
        fs::write(&audio, b"metadata was stale").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let data = runtime.block_on(async {
            read_capped_local_audio_with_capacity_hint(
                tokio::fs::File::open(&audio).await.unwrap(),
                1,
            )
            .await
            .unwrap()
        });

        assert_eq!(data, b"metadata was stale");
        assert!(data.capacity() <= MAX_ENCODED_AUDIO_BYTES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn capped_local_audio_growth_is_geometric_and_never_exceeds_the_cap() {
        let first = next_local_audio_capacity(0, 0, 1)
            .unwrap()
            .expect("an empty buffer needs its first allocation");
        assert_eq!(first, 8 * 1024);
        let second = next_local_audio_capacity(first, first, 1)
            .unwrap()
            .expect("the next full chunk needs a growth step");
        assert_eq!(second, first * 2);
        assert_eq!(
            next_local_audio_capacity(MAX_ENCODED_AUDIO_BYTES, MAX_ENCODED_AUDIO_BYTES, 1)
                .unwrap_err()
                .code,
            shared::error::ErrorCode::InputSaturated
        );
    }

    #[test]
    fn local_audio_errors_do_not_disclose_the_resolved_absolute_path() {
        let source = include_str!("ops.rs");
        let reader = source
            .find("async fn read_capped_local_audio")
            .expect("capped local reader");
        let body = &source[reader..source.find("// ============================================================================\n// Context Operations").unwrap()];

        assert!(body.contains("Failed to read local audio file"));
        assert!(
            !body.contains("path"),
            "reader errors must not format a resolved path"
        );
    }

    #[test]
    fn local_audio_reserves_before_opening_and_rolls_back_on_every_error() {
        let source = include_str!("ops.rs");
        let start = source
            .find("async fn op_inner_audio_load_url_owned")
            .expect("local audio load op");
        let end = source[start..]
            .find("/// Play InnerAudioContext")
            .map(|offset| start + offset)
            .expect("end of local audio load op");
        let body = &source[start..end];
        let open = body.find("open_local_audio_source(vfs, source)").unwrap();
        let read = body.find("read_capped_local_audio(file)").unwrap();
        let reserve = body
            .find("try_reserve_data(MAX_ENCODED_AUDIO_BYTES)")
            .unwrap();
        let send = body.find("send_reserved").unwrap();

        assert!(reserve < open && open < read && read < send);
        assert!(
            body.contains("permit.shrink_to(data.capacity())"),
            "the actual Vec capacity must be the queued-byte charge"
        );
    }

    #[test]
    fn synchronous_audio_string_ops_borrow_then_reserve_then_copy() {
        let source = include_str!("ops.rs");
        for name in [
            "op_audio_set_node_param",
            "op_audio_param_set_value_at_time",
            "op_audio_param_linear_ramp",
            "op_audio_param_exponential_ramp",
            "op_audio_param_set_target",
            "op_audio_param_cancel_scheduled",
            "op_audio_set_oscillator_type",
            "op_audio_set_biquad_filter_type",
            "op_audio_set_wave_shaper_oversample",
            "op_audio_set_panning_model",
            "op_audio_set_distance_model",
            "op_audio_set_panner_scalar",
        ] {
            let start = source.find(&format!("pub fn {name}")).unwrap();
            let body = &source[start
                ..source[start + 1..]
                    .find("#[op2")
                    .map(|end| start + end + 1)
                    .unwrap_or(source.len())];
            assert!(body.contains("#[string]"));
            assert!(body.contains(": &str"), "{name} must borrow its V8 string");
            let validate = body.find("validate_audio_control_string").unwrap();
            let reserve = body.find("try_reserve_data").unwrap();
            let owned = body.find(".to_owned()").unwrap();
            assert!(body.contains("send_reserved"));
            assert!(validate < reserve && reserve < owned, "{name}");
        }
    }

    #[test]
    fn frequency_response_borrows_aligned_f32_bytes_before_queue_admission() {
        let source = include_str!("ops.rs");
        let start = source
            .find("pub async fn op_audio_get_frequency_response")
            .unwrap();
        let body = &source[start
            ..source[start..]
                .find("/// Get current reduction")
                .map(|end| start + end)
                .unwrap()];

        assert!(body.contains("#[buffer] frequencies: JsBuffer"));
        let validate = body.find("validate_frequency_response_bytes").unwrap();
        let reserve = body.find("try_reserve_data").unwrap();
        let convert = body.find("chunks_exact(4)").unwrap();
        let send = body.find("send_reserved").unwrap();
        assert!(validate < reserve && reserve < convert && convert < send);
    }

    #[test]
    fn frequency_response_byte_validation_has_alignment_and_exact_point_boundaries() {
        assert!(validate_frequency_response_bytes(MAX_FREQUENCY_RESPONSE_POINTS * 4).is_ok());
        assert_eq!(
            validate_frequency_response_bytes(MAX_FREQUENCY_RESPONSE_POINTS * 4 + 4)
                .unwrap_err()
                .code,
            shared::error::ErrorCode::InputSaturated
        );
        assert_eq!(
            validate_frequency_response_bytes(3).unwrap_err().code,
            shared::error::ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn audio_buffer_constructor_owns_the_pcm_limit_and_validates_before_reserving() {
        const LIMIT: &str = "64 * 1024 * 1024";
        let buffer = include_str!("00_audio_buffer.js");

        assert!(buffer.contains(LIMIT));
        assert!(!buffer.contains("512 * 1024 * 1024"));
        assert!(buffer.contains(
            "constructor({ numberOfChannels = 1, length, sampleRate }, internal = undefined)"
        ));
        assert!(buffer.contains("op_audio_reserve_buffer"));
        assert!(buffer.contains("op_audio_abort_buffer"));
        assert!(buffer.contains("DECODED_BUFFER_TOKEN"));
        assert!(buffer.contains("internal.token !== DECODED_BUFFER_TOKEN"));
        assert!(!buffer.contains("static _fromDecoded"));
        let reserve = buffer.find("op_audio_reserve_buffer").unwrap();
        let allocate = buffer.find("new ArrayBuffer").unwrap();
        let abort = buffer.find("op_audio_abort_buffer(id)").unwrap();
        assert!(
            reserve < allocate && allocate < abort,
            "reserve/allocate/abort ordering"
        );
        assert!(
            buffer
                .contains("new Float32Array(this.#backing, channel * channelBytes, this.#length)")
        );
    }

    #[test]
    fn remote_audio_url_classification_is_case_insensitive_and_exact() {
        let http = parse_remote_audio_url("HTTP://media.example/a.mp3")
            .unwrap()
            .expect("HTTP URL should be remote");
        assert_eq!(http.scheme(), "http");

        let https = parse_remote_audio_url("https://media.example/b.mp3")
            .unwrap()
            .expect("HTTPS URL should be remote");
        assert_eq!(https.scheme(), "https");

        assert!(parse_remote_audio_url("audio/bgm.mp3").unwrap().is_none());
        assert!(
            parse_remote_audio_url("asset://audio/bgm.mp3")
                .unwrap()
                .is_none()
        );
        assert!(parse_remote_audio_url("http://").is_err());
    }

    #[test]
    fn local_audio_source_is_borrowed_and_copied_only_after_the_size_guard() {
        let source = include_str!("ops.rs");
        let start = source.find("pub fn op_inner_audio_load_url").unwrap();
        let end = source[start..]
            .find("async fn op_inner_audio_load_url_owned")
            .map(|offset| start + offset)
            .unwrap();
        let entry = &source[start..end];

        assert!(entry.contains("#[string] src: &str"));
        assert!(entry.contains("Result<impl std::future::Future"));
        assert!(entry.contains("prepare_inner_audio_src(src)?"));

        let helper_start = source.find("fn prepare_inner_audio_src").unwrap();
        let helper_end = source[helper_start..]
            .find("async fn op_inner_audio_load_url_owned")
            .map(|offset| helper_start + offset)
            .unwrap();
        let helper = &source[helper_start..helper_end];
        let validate = helper.find("validate_audio_control_string").unwrap();
        let copy = helper.find("src.to_owned()").unwrap();
        assert!(validate < copy);
    }

    #[test]
    fn oversized_local_audio_source_does_not_allocate_in_proportion_to_input() {
        let oversized = "x".repeat(8 * 1024 * 1024);
        let before = migo_alloc_probe::thread_counts();
        let error = match prepare_inner_audio_src(&oversized) {
            Ok(_) => panic!("an oversized source unexpectedly passed validation"),
            Err(error) => error,
        };
        let after = migo_alloc_probe::thread_counts();
        let allocated = after.bytes_allocated - before.bytes_allocated;

        assert!(error.to_string().contains("InputSaturated"));
        assert!(
            allocated < 64 * 1024,
            "the native guard allocated {allocated} bytes for an oversized borrowed source"
        );
    }

    #[test]
    fn web_audio_registries_do_not_strongly_retain_contexts_buffers_or_nodes() {
        let context = include_str!("01_audio_context.js");
        let node = include_str!("00_audio_node.js");
        let buffer = include_str!("00_audio_buffer.js");

        assert!(
            !context.contains("const BUFFER_REGISTRY = new Map()"),
            "AudioBuffer PCM must not be pinned by a module-global strong registry"
        );
        assert!(
            context.contains("new WeakRef(this)"),
            "the lifecycle registry must not keep abandoned AudioContexts alive"
        );
        assert!(
            context.contains("new FinalizationRegistry"),
            "abandoned AudioContexts need a native cleanup path"
        );
        assert!(
            !node.contains("NODE_REGISTRY.set"),
            "an unused registry must not pin every AudioNode and its context"
        );
        assert!(
            buffer.contains("new FinalizationRegistry")
                && buffer.contains("op_audio_release_buffer(id)"),
            "unreachable AudioBuffers must release their native retained PCM"
        );
    }

    #[test]
    fn audio_buffers_are_not_context_scoped_and_setters_only_associate_before_start() {
        let source = include_str!("00_buffer_source_node.js");
        let setter = source.find("set buffer(value)").expect("buffer setter");
        let end = source[setter..]
            .find("get loop()")
            .map(|offset| setter + offset)
            .expect("end of buffer setter");
        let body = &source[setter..end];

        assert!(
            !body.contains("_ctxId"),
            "AudioBuffer must be usable in another context"
        );
        assert!(
            !body.contains("op_audio_set_buffer"),
            "pre-start association stays in JS"
        );
        assert!(body.contains("if (value !== null && this.#bufferWasSet)"));
        assert!(body.contains("if (value !== null) this.#bufferWasSet = true"));
        assert!(body.contains("if (!this.#started)"));
        assert!(body.contains("op_audio_set_started_buffer"));
    }

    #[test]
    fn buffer_source_uses_the_new_synchronous_buffer_ops() {
        let source = include_str!("00_buffer_source_node.js");
        assert!(source.contains("op_audio_start_buffer"));
        assert!(source.contains("op_audio_set_started_buffer"));
        assert!(!source.contains("op_audio_set_buffer"));
        assert!(!source.contains("op_audio_start("));

        let start = source
            .find("start(when = 0, offset = 0, duration)")
            .unwrap();
        let body = &source[start..source[start..].find("stop(when = 0)").unwrap() + start];
        let native = body
            .find("op_audio_start_buffer")
            .expect("synchronous native start");
        let commit = body
            .find("this.#started = true")
            .expect("started-state commit");
        let acquire = body
            .find("buffer._acquireBacking()")
            .expect("backing acquisition");
        assert!(
            acquire < native && native < commit,
            "a failed native start must leave the node startable"
        );
    }

    #[test]
    fn web_audio_ids_are_allocated_by_the_host_across_runtime_restarts() {
        let source = include_str!("01_audio_context.js");

        assert!(source.contains("allocateHostCallbackId"));
        assert!(!source.contains("let nextContextId"));
        assert!(!source.contains("let nextNodeId"));
        assert!(!source.contains("nextContextId++"));
        assert!(!source.contains("nextNodeId++"));
        assert!(
            source.matches("allocateHostCallbackId()").count() >= 2,
            "both contexts and nodes must use the host-lifetime allocator"
        );
    }

    #[test]
    fn audio_buffer_finalizer_releases_the_global_buffer_id_directly() {
        let buffer = include_str!("00_audio_buffer.js");
        let context = include_str!("01_audio_context.js");

        assert!(!buffer.contains("PENDING_BUFFER_RELEASES"));
        assert!(!buffer.contains("setTimeout(flushPendingBufferReleases"));
        assert!(buffer.contains("AUDIO_BUFFER_FINALIZER"));
        assert!(buffer.contains("op_audio_release_buffer(id)"));

        assert!(context.contains("op_audio_release_context"));
        assert!(context.contains("const PENDING_CONTEXT_RELEASES = new Set()"));
        assert!(context.contains("setTimeout(flushPendingContextReleases"));
    }

    #[test]
    fn decode_adoption_atomically_takes_the_temporary_native_buffer() {
        let source = include_str!("01_audio_context.js");
        let start = source.find("async decodeAudioData").expect("decode method");
        let end = source[start..]
            .find("  createBuffer(")
            .map(|offset| start + offset)
            .expect("end of decode method");
        let body = &source[start..end];

        let retained = body
            .find("op_audio_decode_audio_data")
            .expect("native decode");
        let take = body
            .find("op_audio_take_decoded_buffer_data(this.#nativeId, info.id)")
            .expect("atomic native buffer take");
        assert!(retained < take);
        assert!(!body.contains("op_audio_release_decoded_buffer"));
    }

    #[test]
    fn buffer_source_enforces_first_non_null_assignment_as_set_once() {
        let source = include_str!("00_buffer_source_node.js");
        let setter = source.find("set buffer(value)").expect("buffer setter");
        let end = source[setter..]
            .find("get loop()")
            .map(|offset| setter + offset)
            .expect("end of setter");
        let body = &source[setter..end];

        assert!(source.contains("#bufferWasSet = false"));
        let guard = body
            .find("value !== null && this.#bufferWasSet")
            .expect("set-once guard");
        let commit = body
            .find("this.#bufferWasSet = true")
            .expect("successful first assignment commit");
        assert!(guard < commit);
        assert!(
            body.contains("value !== null"),
            "null must not consume the one assignment"
        );
    }

    #[test]
    fn create_buffer_is_synchronous_and_has_no_legacy_native_create_or_copy_ops() {
        let context = include_str!("01_audio_context.js");
        let buffer = include_str!("00_audio_buffer.js");
        let create = context
            .find("  createBuffer(")
            .expect("createBuffer method");
        let body =
            &context[create..context[create..].find("  createBufferSource()").unwrap() + create];

        assert!(!body.contains("async createBuffer"));
        assert!(body.contains("return new AudioBuffer({ numberOfChannels, length, sampleRate })"));
        assert!(!context.contains("op_audio_create_buffer("));
        assert!(!buffer.contains("op_audio_copy_to_channel"));
        assert!(!buffer.contains("op_audio_get_channel_data"));
    }

    #[test]
    fn buffer_reacquires_backing_after_native_acquisition_or_freeze() {
        let buffer = include_str!("00_audio_buffer.js");
        assert!(buffer.contains("_acquireBacking()"));
        assert!(buffer.contains("this.#backing = null"));
        assert!(buffer.contains("this.#channelData = null"));
        assert!(buffer.contains("op_audio_materialize_buffer(this.#id)"));
        assert!(buffer.contains("copyToChannel(source, channelNumber, startInChannel = 0)"));
        assert!(buffer.contains("data.set(source.subarray(0, len), start)"));
    }

    #[test]
    fn decode_builds_each_channel_once_without_legacy_get_channel_data() {
        let context = include_str!("01_audio_context.js");
        let start = context.find("async decodeAudioData").unwrap();
        let body = &context[start..context[start..].find("  createBuffer(").unwrap() + start];
        assert_eq!(body.matches("channelData.push(").count(), 0);
        assert!(!body.contains("op_audio_get_channel_data"));
        assert!(body.contains("createDecodedAudioBuffer"));
    }

    #[test]
    fn remote_audio_is_gated_before_it_reaches_the_audio_command_channel() {
        let source = include_str!("ops.rs");
        let start = source
            .find("async fn op_inner_audio_load_url_owned")
            .expect("audio load op");
        let end = source[start..]
            .find("/// Play InnerAudioContext")
            .map(|offset| start + offset)
            .expect("end of audio load op");
        let body = &source[start..end];
        let gate = body
            .find("GateKind::AudioStream")
            .expect("remote audio must use the shared gate");
        let enqueue = body
            .find("AudioCmd::InnerAudioLoadUrl")
            .expect("remote audio command");
        assert!(
            gate < enqueue,
            "policy gate must run before command enqueue"
        );
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
    fn resolve_local_src_preserves_user_virtual_path_for_descriptor_safe_open() {
        let base = make_temp_dir("migo_audio_vfs");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");

        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&tmp).unwrap();

        let vfs = VirtualFS::new(code.clone(), user.clone(), cache, tmp);
        let source =
            resolve_local_src(code.to_str(), Some(&vfs), "/user/gamecaches/audio/bgm.mp3").unwrap();

        assert_eq!(
            source,
            LocalAudioSource::Sandboxed {
                virtual_path: "/user/gamecaches/audio/bgm.mp3".to_string(),
            }
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn resolve_local_src_maps_relative_paths_to_the_code_virtual_root() {
        let base = make_temp_dir("migo_audio_relative_vfs");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");
        for directory in [&code, &user, &cache, &tmp] {
            fs::create_dir_all(directory).unwrap();
        }

        let vfs = VirtualFS::new(code.clone(), user, cache, tmp);
        let source = resolve_local_src(code.to_str(), Some(&vfs), "audio/bgm.mp3").unwrap();
        assert_eq!(
            source,
            LocalAudioSource::Sandboxed {
                virtual_path: "/code/audio/bgm.mp3".to_string(),
            }
        );

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn resolve_local_src_rejects_absolute_path_outside_sandbox() {
        let base = make_temp_dir("migo_audio_escape");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");

        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&tmp).unwrap();

        let vfs = VirtualFS::new(code.clone(), user, cache, tmp);
        // An absolute path outside every virtual root (/code, /user, /cache, /tmp)
        // must NOT resolve to a real filesystem path — that would escape the sandbox.
        let result = resolve_local_src(code.to_str(), Some(&vfs), "/etc/passwd").unwrap_err();
        assert_eq!(result.to_string(), "Local audio path is not permitted");
        assert!(!result.to_string().contains("/etc/passwd"));
        assert!(!result.to_string().contains(&base.to_string_lossy()[..]));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn local_audio_vfs_branch_opens_the_descriptor_instead_of_reopening_a_resolved_path() {
        let source = include_str!("ops.rs");
        let start = source.find("async fn open_local_audio_source").unwrap();
        let end = source[start..]
            .find("/// Load audio from URL or local path")
            .map(|offset| start + offset)
            .unwrap();
        let body = &source[start..end];

        assert!(body.contains("open_regular_for_read"));
        assert!(body.contains("spawn_blocking"));
        assert!(!body.contains("vfs.resolve"));
    }

    #[test]
    fn local_audio_source_reads_through_the_already_open_vfs_descriptor() {
        let base = make_temp_dir("migo_audio_strict_vfs_open");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");
        for directory in [&code, &user, &cache, &tmp] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(user.join("clip.bin"), b"sandboxed-audio").unwrap();

        let vfs = std::sync::Arc::new(VirtualFS::new(code.clone(), user, cache, tmp));
        let source = resolve_local_src(code.to_str(), Some(&vfs), "/user/clip.bin").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let bytes = runtime.block_on(async {
            let file = open_local_audio_source(Some(vfs), source).await.unwrap();
            read_capped_local_audio(file).await.unwrap()
        });

        assert_eq!(bytes, b"sandboxed-audio");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn vendor_specific_local_uri_schemes_are_rejected_without_echoing_them() {
        let base = make_temp_dir("migo_audio_vendor_uri");
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");
        for directory in [&code, &user, &cache, &tmp] {
            fs::create_dir_all(directory).unwrap();
        }

        let vfs = std::sync::Arc::new(VirtualFS::new(code.clone(), user, cache, tmp));
        let original = "vendor-file://user/audio/clip.bin";
        let source = resolve_local_src(code.to_str(), Some(&vfs), original).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(open_local_audio_source(Some(vfs), source))
            .unwrap_err();

        assert_eq!(error.to_string(), "Failed to open local audio file");
        assert!(!error.to_string().contains(original));
        let _ = fs::remove_dir_all(base);
    }
}
