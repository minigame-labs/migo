#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use shared::audio_resources::AudioSnapshot;
use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::audio_cmd::{AudioBufferId, AudioContextId, AudioContextState, AudioNodeId};

use crate::decoder::DecodedAudio;
use crate::limits::{PcmBudget, RetainedAudio};
use crate::nodes::{
    AudioNodeProcessor, BufferSourceNode, DestinationNode, GainNode, NodeConnection,
};

/// Special node ID for the destination node
pub const DESTINATION_NODE_ID: AudioNodeId = 0;

/// An AudioContext manages audio resources and the audio graph.
///
/// Nodes are stored in a generic HashMap and processed in topological order
/// determined by the connection graph. This supports arbitrary node types
/// and connection chains (e.g., source → filter → gain → destination).
pub struct AudioContext {
    id: AudioContextId,
    state: AudioContextState,
    sample_rate: u32,
    channels: u32,

    // Resources
    buffers: HashMap<AudioBufferId, Arc<RetainedAudio>>,
    next_buffer_id: AudioBufferId,
    pcm_budget: PcmBudget,

    // Generic node storage
    nodes: HashMap<AudioNodeId, Box<dyn AudioNodeProcessor>>,

    // Graph connections
    connections: Vec<NodeConnection>,

    // Topologically sorted processing order (cached, invalidated on graph change)
    processing_order: Vec<AudioNodeId>,
    graph_dirty: bool,

    // Pre-built reverse adjacency: dst_node → [src_nodes] for O(1) input lookup
    input_adjacency: HashMap<AudioNodeId, Vec<AudioNodeId>>,

    // Processing buffers: per-node output buffers keyed by node ID
    node_buffers: HashMap<AudioNodeId, Vec<f32>>,
    // Scratch buffer for mixing multiple inputs
    mix_buffer: Vec<f32>,

    // Frames processed for sample-accurate currentTime (W3C spec)
    frames_processed: u64,
}

impl AudioContext {
    /// Default capacity for audio resources.
    const DEFAULT_BUFFER_CAPACITY: usize = 8;
    const DEFAULT_NODE_CAPACITY: usize = 16;

    pub fn new(id: AudioContextId, sample_rate: u32, channels: u32) -> Self {
        Self::new_with_pcm_budget(id, sample_rate, channels, PcmBudget::for_context())
    }

    fn new_with_pcm_budget(
        id: AudioContextId,
        sample_rate: u32,
        channels: u32,
        pcm_budget: PcmBudget,
    ) -> Self {
        let mut nodes: HashMap<AudioNodeId, Box<dyn AudioNodeProcessor>> =
            HashMap::with_capacity(Self::DEFAULT_NODE_CAPACITY);

        // Always create the destination node at ID 0
        nodes.insert(
            DESTINATION_NODE_ID,
            Box::new(DestinationNode::new(DESTINATION_NODE_ID, channels)),
        );

        Self {
            id,
            state: AudioContextState::Running,
            sample_rate,
            channels,
            buffers: HashMap::with_capacity(Self::DEFAULT_BUFFER_CAPACITY),
            next_buffer_id: 1,
            pcm_budget,
            nodes,
            connections: Vec::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            processing_order: Vec::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            graph_dirty: true,
            input_adjacency: HashMap::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            node_buffers: HashMap::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            mix_buffer: Vec::new(),
            frames_processed: 0,
        }
    }

    pub fn id(&self) -> AudioContextId {
        self.id
    }

    pub fn state(&self) -> AudioContextState {
        self.state
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Get the current time based on frames processed (W3C AudioContext spec).
    ///
    /// Unlike wall-clock time, this pauses when the context is suspended
    /// and accurately tracks the audio output position.
    pub fn current_time(&self) -> f64 {
        self.frames_processed as f64 / self.sample_rate.max(1) as f64
    }

    pub fn suspend(&mut self) {
        self.state = AudioContextState::Suspended;
    }

    pub fn resume(&mut self) {
        if self.state == AudioContextState::Suspended {
            self.state = AudioContextState::Running;
        }
    }

    pub fn close(&mut self) {
        self.state = AudioContextState::Closed;
        self.buffers.clear();
        self.nodes.clear();
        self.connections.clear();
        self.processing_order.clear();
        self.input_adjacency.clear();
        self.node_buffers.clear();
    }

    /// Check if this context has any active source nodes (playing or scheduled).
    ///
    /// Used by the power manager to avoid keeping the audio thread at high
    /// tick rate when a context is Running but has no audio to produce.
    pub fn has_active_sources(&self) -> bool {
        if self.state != AudioContextState::Running {
            return false;
        }
        self.nodes
            .values()
            .any(|n| n.is_source() && !n.is_finished())
    }

    // ==================== Buffer Management ====================

    pub fn add_buffer(&mut self, audio: DecodedAudio) -> EngineResult<AudioBufferId> {
        let (id, next_id) = self.buffer_id_candidate()?;
        let retained = RetainedAudio::try_new(audio, &self.pcm_budget)?;
        self.buffers.insert(id, Arc::new(retained));
        self.next_buffer_id = next_id;
        Ok(id)
    }

    pub fn get_buffer(&self, id: AudioBufferId) -> Option<Arc<RetainedAudio>> {
        self.buffers.get(&id).cloned()
    }

    pub fn remove_buffer(&mut self, id: AudioBufferId) -> bool {
        self.buffers.remove(&id).is_some()
    }

    fn buffer_id_candidate(&self) -> EngineResult<(AudioBufferId, AudioBufferId)> {
        let id = self.next_buffer_id;
        if id >= i32::MAX as AudioBufferId {
            return Err(shared::error::EngineError::from_detail(
                shared::error::ErrorCode::InputSaturated,
                "AudioBuffer id space exhausted at the V8 Smi boundary",
            ));
        }
        let next_id = id.checked_add(1).ok_or_else(|| {
            shared::error::EngineError::from_detail(
                shared::error::ErrorCode::InputSaturated,
                "AudioBuffer id space exhausted",
            )
        })?;
        if self.buffers.contains_key(&id) {
            return Err(shared::error::EngineError::from_detail(
                shared::error::ErrorCode::InputSaturated,
                format!("AudioBuffer id {id} is still in use"),
            ));
        }
        Ok((id, next_id))
    }

    /// Create an empty buffer with the given parameters.
    ///
    /// Validates channel count, sample rate and total PCM size (with checked
    /// arithmetic) before allocating, so an overflowing or budget-busting
    /// request is rejected instead of panicking / wrapping to a bogus size.
    pub fn create_empty_buffer(
        &mut self,
        channels: u32,
        length: u32,
        sample_rate: u32,
    ) -> EngineResult<AudioBufferId> {
        let bytes = crate::limits::validated_buffer_alloc_bytes(channels, length, sample_rate)?;
        let (id, next_id) = self.buffer_id_candidate()?;
        // Claim both aggregate budgets before asking the allocator for the Vec.
        let mut permit = self.pcm_budget.reserve(bytes)?;
        let sample_count = bytes / std::mem::size_of::<f32>();
        let samples = crate::limits::try_allocate_zeroed_pcm(sample_count)?;
        let capacity_bytes = crate::limits::pcm_bytes(samples.capacity())?;
        permit.try_grow_to(capacity_bytes)?;
        let audio = DecodedAudio {
            samples,
            sample_rate,
            channels,
        };
        self.buffers
            .insert(id, Arc::new(RetainedAudio::from_reserved(audio, permit)));
        self.next_buffer_id = next_id;
        Ok(id)
    }

    /// Number of channels in a buffer (0 if not found).
    pub fn buffer_channels(&self, buffer_id: AudioBufferId) -> Option<u32> {
        self.buffers.get(&buffer_id).map(|b| b.channels)
    }

    /// Get raw channel data from a buffer
    pub fn get_channel_data(&self, buffer_id: AudioBufferId, channel: u32) -> Option<Vec<f32>> {
        let buffer = self.buffers.get(&buffer_id)?;
        if channel >= buffer.channels {
            return None;
        }
        let frame_count = buffer.frame_count();
        let channels = buffer.channels as usize;
        let ch = channel as usize;
        let mut data = Vec::with_capacity(frame_count);
        for frame in 0..frame_count {
            data.push(buffer.samples[frame * channels + ch]);
        }
        Some(data)
    }

    /// Return all channels in one flat channel-major vector.
    ///
    /// The native buffer remains interleaved; this allocates exactly one
    /// result vector and fills it without constructing one `Vec` per channel.
    /// Reservation is fallible so malformed dimensions or allocator failure
    /// become structured errors before any output is built.
    pub fn take_decoded_buffer_data(
        &mut self,
        buffer_id: AudioBufferId,
    ) -> EngineResult<Option<Vec<f32>>> {
        let buffer = match self.buffers.remove(&buffer_id) {
            Some(buffer) => buffer,
            None => return Ok(None),
        };
        let channels = usize::try_from(buffer.channels).map_err(|_| {
            EngineError::from_detail(
                ErrorCode::InvalidArgument,
                "channel count does not fit usize",
            )
        })?;
        let frames = buffer.frame_count();
        let sample_count = channels.checked_mul(frames).ok_or_else(|| {
            EngineError::from_detail(
                ErrorCode::InvalidArgument,
                "audio channel data sample count overflow",
            )
        })?;

        let mut output = Vec::new();
        output.try_reserve_exact(sample_count).map_err(|_| {
            EngineError::from_detail(
                ErrorCode::OutOfMemory,
                "audio channel data allocation failed",
            )
        })?;
        for channel in 0..channels {
            for frame in 0..frames {
                let sample_index = frame
                    .checked_mul(channels)
                    .and_then(|base| base.checked_add(channel))
                    .ok_or_else(|| {
                        EngineError::from_detail(
                            ErrorCode::InvalidArgument,
                            "audio channel data index overflow",
                        )
                    })?;
                output.push(buffer.samples[sample_index]);
            }
        }
        Ok(Some(output))
    }

    /// Copy data into a specific channel of a buffer (copy-on-write via Arc::make_mut).
    pub fn copy_to_channel(
        &mut self,
        buffer_id: AudioBufferId,
        data: &[f32],
        channel: u32,
        start_frame: u32,
    ) -> EngineResult<bool> {
        let pcm_budget = self.pcm_budget.clone();
        let buffer = match self.buffers.get_mut(&buffer_id) {
            Some(b) => b,
            None => return Ok(false),
        };

        if channel >= buffer.channels {
            return Ok(false);
        }

        let channels = buffer.channels as usize;
        let frame_count = buffer.frame_count();
        let start = start_frame as usize;
        let ch = channel as usize;

        // Clamp copy length to buffer bounds
        let copy_len = data.len().min(frame_count.saturating_sub(start));
        if copy_len == 0 {
            return Ok(true); // Nothing to copy, but not an error
        }

        // A source may hold the old Arc. Reserve and allocate the COW copy
        // fallibly before replacing the map entry, keeping the old PCM intact
        // if either aggregate budget refuses the duplicate.
        if Arc::get_mut(buffer).is_none() {
            let cloned = buffer.try_clone_with_budget(&pcm_budget)?;
            *buffer = Arc::new(cloned);
        }
        let audio = Arc::get_mut(buffer)
            .ok_or_else(|| {
                shared::error::EngineError::from_detail(
                    shared::error::ErrorCode::Internal,
                    "retained PCM did not become uniquely owned after copy-on-write",
                )
            })?
            .audio_mut();

        for i in 0..copy_len {
            let sample_idx = (start + i) * channels + ch;
            if sample_idx < audio.samples.len() {
                audio.samples[sample_idx] = data[i];
            }
        }

        Ok(true)
    }

    // ==================== Node Management ====================

    /// Create a buffer source node with JS-provided node_id
    pub fn create_buffer_source(&mut self, node_id: AudioNodeId) {
        tracing::trace!("create_buffer_source: node_id={}", node_id);
        self.nodes.insert(
            node_id,
            Box::new(BufferSourceNode::new(node_id, self.sample_rate)),
        );
        self.graph_dirty = true;
    }

    /// Create a gain node with JS-provided node_id
    pub fn create_gain(&mut self, node_id: AudioNodeId) {
        tracing::trace!("create_gain: node_id={}", node_id);
        self.nodes.insert(node_id, Box::new(GainNode::new(node_id)));
        self.graph_dirty = true;
    }

    /// Add a generic node (used by future node types)
    pub fn add_node(&mut self, node: Box<dyn AudioNodeProcessor>) {
        let node_id = node.id();
        tracing::trace!("add_node: node_id={}, type={:?}", node_id, node.node_type());
        self.nodes.insert(node_id, node);
        self.graph_dirty = true;
    }

    /// Downcast and operate on a specific node type.
    /// Used for type-specific operations (e.g., set_buffer on BufferSourceNode).
    pub fn with_node_typed<T: AudioNodeProcessor + 'static, R>(
        &mut self,
        node_id: AudioNodeId,
        f: impl FnOnce(&mut T) -> R,
    ) -> Option<R> {
        let node = self.nodes.get_mut(&node_id)?;
        // Use Any for downcasting
        let any = node.as_any_mut();
        let typed = any.downcast_mut::<T>()?;
        Some(f(typed))
    }

    pub fn set_buffer(&mut self, node_id: AudioNodeId, buffer_id: Option<AudioBufferId>) -> bool {
        tracing::trace!("set_buffer: node_id={}, buffer_id={buffer_id:?}", node_id);
        let buffer = match buffer_id {
            Some(buffer_id) => match self.buffers.get(&buffer_id) {
                Some(buffer) => Some(Arc::clone(buffer)),
                None => {
                    tracing::warn!("set_buffer: buffer {} not found", buffer_id);
                    return false;
                }
            },
            None => None,
        };

        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.set_buffer(buffer);
                return true;
            }
        }
        tracing::warn!("set_buffer: source node {} not found", node_id);
        false
    }

    /// Replace or clear the JS-owned snapshot on a source node without
    /// copying its interleaved PCM.
    pub fn set_started_buffer(
        &mut self,
        node_id: AudioNodeId,
        buffer: Option<Arc<AudioSnapshot>>,
    ) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.set_snapshot(buffer);
                return true;
            }
        }
        false
    }

    pub fn start_source(
        &mut self,
        node_id: AudioNodeId,
        when: f64,
        offset: f64,
        duration: Option<f64>,
    ) -> bool {
        tracing::trace!(
            "start_source: node_id={}, when={}, offset={}, duration={:?}",
            node_id,
            when,
            offset,
            duration
        );
        let current_time = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.start(when, offset, duration, current_time);
                tracing::trace!("start_source: node started");
                return true;
            }
        }
        tracing::warn!("start_source: node {} not found", node_id);
        false
    }

    pub fn stop_source(&mut self, node_id: AudioNodeId, when: f64) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.stop(when);
                return true;
            }
        }
        false
    }

    /// If the node exists and has already finished (e.g. `stop(when <= 0)`
    /// finishes a buffer source immediately), remove it from every per-node
    /// structure (node map, output buffer, connections) and return `true`.
    ///
    /// Lets the audio thread fully clean up an immediately-finished node now,
    /// rather than waiting for the next `process()` sweep — which never runs
    /// while the context is suspended.
    pub fn remove_finished_node(&mut self, node_id: AudioNodeId) -> bool {
        let finished = self
            .nodes
            .get(&node_id)
            .map(|n| n.is_finished())
            .unwrap_or(false);
        if !finished {
            return false;
        }
        self.nodes.remove(&node_id);
        self.node_buffers.remove(&node_id);
        self.connections
            .retain(|c| c.src != node_id && c.dst != node_id);
        self.graph_dirty = true;
        true
    }

    pub fn set_loop(&mut self, node_id: AudioNodeId, enabled: bool, start: f64, end: f64) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.set_loop(enabled, start, end);
                return true;
            }
        }
        false
    }

    pub fn set_playback_rate(&mut self, node_id: AudioNodeId, rate: f32) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                source.set_playback_rate(rate);
                return true;
            }
        }
        false
    }

    pub fn set_gain(&mut self, node_id: AudioNodeId, value: f32) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(gain) = any.downcast_mut::<GainNode>() {
                gain.set_gain(value);
                return true;
            }
        }
        false
    }

    /// Set an AudioParam value on a node by name
    pub fn set_node_param(&mut self, node_id: AudioNodeId, param_name: &str, value: f32) -> bool {
        let now = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_value_now(value, now);
                return true;
            }
        }
        false
    }

    /// Schedule AudioParam automation on a node
    pub fn param_set_value_at_time(
        &mut self,
        node_id: AudioNodeId,
        param_name: &str,
        value: f32,
        time: f64,
    ) -> bool {
        let now = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_value_at_time(value, time);
                param.gc_events(now);
                return true;
            }
        }
        false
    }

    pub fn param_linear_ramp(
        &mut self,
        node_id: AudioNodeId,
        param_name: &str,
        value: f32,
        end_time: f64,
    ) -> bool {
        let now = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.linear_ramp_to_value_at_time(value, end_time);
                param.gc_events(now);
                return true;
            }
        }
        false
    }

    pub fn param_exponential_ramp(
        &mut self,
        node_id: AudioNodeId,
        param_name: &str,
        value: f32,
        end_time: f64,
    ) -> bool {
        let now = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.exponential_ramp_to_value_at_time(value, end_time);
                param.gc_events(now);
                return true;
            }
        }
        false
    }

    pub fn param_set_target(
        &mut self,
        node_id: AudioNodeId,
        param_name: &str,
        target: f32,
        start_time: f64,
        time_constant: f64,
    ) -> bool {
        let now = self.current_time();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_target_at_time(target, start_time, time_constant);
                param.gc_events(now);
                return true;
            }
        }
        false
    }

    pub fn param_cancel_scheduled(
        &mut self,
        node_id: AudioNodeId,
        param_name: &str,
        cancel_time: f64,
    ) -> bool {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.cancel_scheduled_values(cancel_time);
                return true;
            }
        }
        false
    }

    // ==================== Graph ====================

    pub fn connect(&mut self, src: AudioNodeId, dst: AudioNodeId) {
        tracing::trace!("connect: src={}, dst={}", src, dst);
        // Avoid duplicate connections
        if !self
            .connections
            .iter()
            .any(|c| c.src == src && c.dst == dst)
        {
            self.connections.push(NodeConnection { src, dst });
            self.graph_dirty = true;
            tracing::trace!(
                "connect: added connection, total={}",
                self.connections.len()
            );
        }
    }

    pub fn disconnect(&mut self, node_id: AudioNodeId, dst: Option<AudioNodeId>) {
        let old_len = self.connections.len();
        self.connections.retain(|c| {
            if let Some(dst_id) = dst {
                !(c.src == node_id && c.dst == dst_id)
            } else {
                c.src != node_id
            }
        });
        if self.connections.len() != old_len {
            self.graph_dirty = true;
        }
    }

    /// Rebuild the processing order using topological sort (Kahn's algorithm)
    /// and pre-build the input adjacency map for O(1) input lookup during processing.
    fn rebuild_processing_order(&mut self) {
        tracing::trace!(
            "rebuild_processing_order: connections={:?}",
            self.connections
        );
        self.processing_order.clear();
        self.input_adjacency.clear();

        // Build adjacency list and in-degree count
        let mut in_degree: HashMap<AudioNodeId, usize> = HashMap::new();
        let mut adj: HashMap<AudioNodeId, Vec<AudioNodeId>> = HashMap::new();

        // Initialize all nodes with in-degree 0
        for &node_id in self.nodes.keys() {
            in_degree.insert(node_id, 0);
            adj.entry(node_id).or_default();
        }

        // Count in-degrees from connections and build reverse adjacency (dst → [src])
        for conn in &self.connections {
            if self.nodes.contains_key(&conn.src) && self.nodes.contains_key(&conn.dst) {
                *in_degree.entry(conn.dst).or_default() += 1;
                adj.entry(conn.src).or_default().push(conn.dst);
                self.input_adjacency
                    .entry(conn.dst)
                    .or_default()
                    .push(conn.src);
            }
        }

        // Start with nodes that have no incoming edges (source nodes)
        let mut queue: Vec<AudioNodeId> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        // Sort for deterministic order
        queue.sort_unstable();

        while let Some(node_id) = queue.pop() {
            // Skip destination — it's always processed last
            if node_id == DESTINATION_NODE_ID {
                continue;
            }
            self.processing_order.push(node_id);

            if let Some(neighbors) = adj.get(&node_id) {
                for &next_id in neighbors {
                    if let Some(deg) = in_degree.get_mut(&next_id) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(next_id);
                        }
                    }
                }
            }
        }

        // Always process destination last
        self.processing_order.push(DESTINATION_NODE_ID);

        tracing::trace!(
            "rebuild_processing_order: order={:?}",
            self.processing_order
        );
        self.graph_dirty = false;
    }

    // ==================== Processing ====================

    /// Process audio and **add** to the output buffer.
    ///
    /// Uses topological sort to process nodes in dependency order.
    /// Each node receives its upstream mixed inputs and writes to its own buffer.
    /// The destination node's output is **added** (not copied) to the final output,
    /// allowing multiple AudioContexts to be mixed together by the caller.
    ///
    /// The caller must zero the output buffer before the first context's process() call.
    ///
    /// Returns the ids of source nodes that finished during this block, so the
    /// audio thread can drop its `node → context` index entries for them.
    pub fn process(&mut self, output: &mut [f32]) -> Vec<AudioNodeId> {
        if self.state != AudioContextState::Running {
            // Don't touch output — other contexts may have already written to it
            return Vec::new();
        }

        // Rebuild processing order if graph changed
        if self.graph_dirty {
            self.rebuild_processing_order();
        }

        let buffer_size = output.len();
        let current_time = self.current_time();
        let sample_rate = self.sample_rate;

        // Ensure mix buffer is large enough
        if self.mix_buffer.len() < buffer_size {
            self.mix_buffer.resize(buffer_size, 0.0);
        }

        // Process each node in topological order (index-based to avoid clone)
        let order_len = self.processing_order.len();
        for order_idx in 0..order_len {
            let node_id = self.processing_order[order_idx];

            // Gather mixed input from upstream using pre-built adjacency (O(inputs) not O(connections))
            self.mix_buffer[..buffer_size].fill(0.0);
            let mut has_input = false;

            if let Some(inputs) = self.input_adjacency.get(&node_id) {
                for &src_id in inputs {
                    if let Some(src_buf) = self.node_buffers.get(&src_id) {
                        let len = src_buf.len().min(buffer_size);
                        for i in 0..len {
                            self.mix_buffer[i] += src_buf[i];
                        }
                        has_input = true;
                    }
                }
            }

            // Process the node
            let node_buf = self
                .node_buffers
                .entry(node_id)
                .or_insert_with(|| vec![0.0f32; buffer_size]);
            if node_buf.len() < buffer_size {
                node_buf.resize(buffer_size, 0.0);
            }
            node_buf[..buffer_size].fill(0.0);

            if let Some(node) = self.nodes.get_mut(&node_id) {
                let input = if has_input {
                    &self.mix_buffer[..buffer_size]
                } else {
                    &[] as &[f32]
                };

                node.process(
                    input,
                    &mut node_buf[..buffer_size],
                    sample_rate,
                    self.channels,
                    current_time,
                );
            }
        }

        // Add destination node's output to final output (additive for multi-context mixing)
        if let Some(dest_buf) = self.node_buffers.get(&DESTINATION_NODE_ID) {
            let len = dest_buf.len().min(buffer_size);
            for i in 0..len {
                output[i] += dest_buf[i];
            }
        }

        // Soft clip only if needed (check max amplitude first)
        let mut needs_clip = false;
        for &sample in output.iter() {
            if sample > 1.0 || sample < -1.0 {
                needs_clip = true;
                break;
            }
        }

        if needs_clip {
            for sample in output.iter_mut() {
                if *sample > 1.0 {
                    *sample = 1.0 - 1.0 / (*sample + 1.0);
                } else if *sample < -1.0 {
                    *sample = -1.0 + 1.0 / (-*sample + 1.0);
                }
            }
        }

        // Track processed frames for sample-accurate currentTime
        let frames = buffer_size / self.channels.max(1) as usize;
        self.frames_processed += frames as u64;

        // Clean up finished source nodes. A naturally-ended one-shot source
        // must be removed from *every* per-node structure — the node map, its
        // output buffer, and any graph connections — and its id reported so the
        // audio thread drops its node→context index entry. Missing any of these
        // leaks one entry per fired sound effect.
        let mut finished: Vec<AudioNodeId> = Vec::new();
        for (&id, node) in self.nodes.iter() {
            if node.is_finished() {
                finished.push(id);
            }
        }
        if !finished.is_empty() {
            for &id in &finished {
                self.nodes.remove(&id);
                self.node_buffers.remove(&id);
            }
            self.connections
                .retain(|c| !finished.contains(&c.src) && !finished.contains(&c.dst));
            self.graph_dirty = true; // processing_order + input_adjacency rebuilt next block
        }

        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{PcmBudget, PcmUsage, PcmUsageSnapshot};

    fn budgeted_context(
        context_bytes: usize,
        context_buffers: usize,
        process: Arc<PcmUsage>,
    ) -> (AudioContext, Arc<PcmUsage>) {
        let context = Arc::new(PcmUsage::new(context_bytes, context_buffers));
        let budget = PcmBudget::new(Arc::clone(&context), process);
        (
            AudioContext::new_with_pcm_budget(1, 48_000, 2, budget),
            context,
        )
    }

    #[test]
    fn all_channel_data_is_one_channel_major_flat_buffer() {
        let mut ctx = AudioContext::new(1, 48_000, 2);
        let id = ctx
            .add_buffer(DecodedAudio {
                samples: vec![1.0, 10.0, 2.0, 20.0, 3.0, 30.0],
                sample_rate: 48_000,
                channels: 2,
            })
            .unwrap();

        assert_eq!(
            ctx.take_decoded_buffer_data(id).unwrap(),
            Some(vec![1.0, 2.0, 3.0, 10.0, 20.0, 30.0])
        );
        assert!(ctx.get_buffer(id).is_none());
        assert_eq!(ctx.take_decoded_buffer_data(id).unwrap(), None);
    }

    #[test]
    fn retained_pcm_enforces_per_context_bytes_and_count() {
        let process = Arc::new(PcmUsage::new(1024, 16));
        let (mut ctx, usage) = budgeted_context(16, 1, process);
        let id = ctx
            .create_empty_buffer(1, 4, 48_000)
            .expect("exact context byte and count budget");
        assert_eq!(
            usage.snapshot(),
            PcmUsageSnapshot {
                bytes: 16,
                buffers: 1
            }
        );

        let error = ctx
            .create_empty_buffer(1, 1, 48_000)
            .expect_err("second retained buffer exceeds count and bytes");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert_eq!(
            usage.snapshot(),
            PcmUsageSnapshot {
                bytes: 16,
                buffers: 1
            }
        );
        assert!(ctx.get_buffer(id).is_some());
    }

    #[test]
    fn retained_pcm_enforces_process_budget_across_contexts() {
        let process = Arc::new(PcmUsage::new(16, 1));
        let (mut first, first_usage) = budgeted_context(32, 2, Arc::clone(&process));
        let (mut second, second_usage) = budgeted_context(32, 2, Arc::clone(&process));
        first
            .create_empty_buffer(1, 4, 48_000)
            .expect("first context fills process budget");

        let error = second
            .create_empty_buffer(1, 1, 48_000)
            .expect_err("second context exceeds process budget");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert_eq!(first_usage.snapshot().bytes, 16);
        assert_eq!(second_usage.snapshot(), PcmUsageSnapshot::default());
        assert_eq!(
            process.snapshot(),
            PcmUsageSnapshot {
                bytes: 16,
                buffers: 1
            }
        );
    }

    #[test]
    fn decoded_pcm_is_charged_by_vector_capacity_not_logical_length() {
        let mut samples = Vec::with_capacity(8);
        samples.push(0.0);
        let retained_bytes = samples.capacity() * std::mem::size_of::<f32>();
        assert!(retained_bytes > samples.len() * std::mem::size_of::<f32>());

        let process = Arc::new(PcmUsage::new(1024, 8));
        let (mut rejected, rejected_usage) =
            budgeted_context(retained_bytes - 1, 8, Arc::clone(&process));
        let error = rejected
            .add_buffer(DecodedAudio {
                samples,
                sample_rate: 48_000,
                channels: 1,
            })
            .expect_err("spare Vec capacity is retained heap and must not bypass the budget");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert_eq!(rejected_usage.snapshot(), PcmUsageSnapshot::default());

        let mut exact_samples = Vec::with_capacity(8);
        exact_samples.push(0.0);
        let exact_bytes = exact_samples.capacity() * std::mem::size_of::<f32>();
        let (mut accepted, accepted_usage) =
            budgeted_context(exact_bytes, 8, Arc::new(PcmUsage::new(1024, 8)));
        accepted
            .add_buffer(DecodedAudio {
                samples: exact_samples,
                sample_rate: 48_000,
                channels: 1,
            })
            .expect("exact retained capacity must fit");
        assert_eq!(accepted_usage.snapshot().bytes, exact_bytes);
    }

    #[test]
    fn releasing_map_entry_keeps_budget_until_source_clears_last_arc() {
        let process = Arc::new(PcmUsage::new(128, 8));
        let (mut ctx, usage) = budgeted_context(128, 8, process);
        let id = ctx.create_empty_buffer(1, 4, 48_000).unwrap();
        ctx.create_buffer_source(10);
        assert!(ctx.set_buffer(10, Some(id)));

        assert!(ctx.remove_buffer(id));
        assert_eq!(
            usage.snapshot(),
            PcmUsageSnapshot {
                bytes: 16,
                buffers: 1
            }
        );

        assert!(ctx.set_buffer(10, None));
        assert_eq!(usage.snapshot(), PcmUsageSnapshot::default());
    }

    #[test]
    fn replacing_source_buffer_releases_the_replaced_last_arc() {
        let process = Arc::new(PcmUsage::new(128, 8));
        let (mut ctx, usage) = budgeted_context(128, 8, process);
        let first = ctx.create_empty_buffer(1, 4, 48_000).unwrap();
        let second = ctx.create_empty_buffer(1, 2, 48_000).unwrap();
        ctx.create_buffer_source(10);
        assert!(ctx.set_buffer(10, Some(first)));
        assert!(ctx.remove_buffer(first));

        assert!(ctx.set_buffer(10, Some(second)));
        assert_eq!(
            usage.snapshot(),
            PcmUsageSnapshot {
                bytes: 8,
                buffers: 1
            }
        );
    }

    #[test]
    fn copy_on_write_reserves_before_cloning_shared_pcm() {
        let process = Arc::new(PcmUsage::new(16, 1));
        let (mut ctx, usage) = budgeted_context(16, 1, process);
        let id = ctx.create_empty_buffer(1, 4, 48_000).unwrap();
        ctx.create_buffer_source(10);
        assert!(ctx.set_buffer(10, Some(id)));

        let error = ctx
            .copy_to_channel(id, &[1.0], 0, 0)
            .expect_err("COW clone must reserve another retained allocation first");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert_eq!(
            usage.snapshot(),
            PcmUsageSnapshot {
                bytes: 16,
                buffers: 1
            }
        );
        assert_eq!(ctx.get_channel_data(id, 0).unwrap()[0], 0.0);

        assert!(ctx.set_buffer(10, None));
        assert!(ctx.copy_to_channel(id, &[1.0], 0, 0).unwrap());
        assert_eq!(ctx.get_channel_data(id, 0).unwrap()[0], 1.0);
    }

    #[test]
    fn copy_on_write_reserves_at_least_the_original_vector_capacity() {
        let mut samples = Vec::with_capacity(8);
        samples.push(0.0);
        let retained_bytes = samples.capacity() * std::mem::size_of::<f32>();
        let process = Arc::new(PcmUsage::new(retained_bytes * 2 - 1, 8));
        let (mut ctx, usage) = budgeted_context(retained_bytes * 2 - 1, 8, Arc::clone(&process));
        let id = ctx
            .add_buffer(DecodedAudio {
                samples,
                sample_rate: 48_000,
                channels: 1,
            })
            .unwrap();
        ctx.create_buffer_source(10);
        assert!(ctx.set_buffer(10, Some(id)));

        let error = ctx
            .copy_to_channel(id, &[1.0], 0, 0)
            .expect_err("COW must conservatively reserve the original allocation capacity");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert_eq!(usage.snapshot().bytes, retained_bytes);
        assert_eq!(process.snapshot().bytes, retained_bytes);
        assert_eq!(ctx.get_channel_data(id, 0).unwrap()[0], 0.0);
    }

    #[test]
    fn copy_on_write_shrinks_pre_reservation_to_the_clone_actual_capacity() {
        let mut samples = Vec::with_capacity(8);
        samples.push(0.0);
        let original_bytes = samples.capacity() * std::mem::size_of::<f32>();
        let process = Arc::new(PcmUsage::new(original_bytes * 2, 8));
        let (mut ctx, usage) = budgeted_context(original_bytes * 2, 8, Arc::clone(&process));
        let id = ctx
            .add_buffer(DecodedAudio {
                samples,
                sample_rate: 48_000,
                channels: 1,
            })
            .unwrap();
        ctx.create_buffer_source(10);
        assert!(ctx.set_buffer(10, Some(id)));

        assert!(ctx.copy_to_channel(id, &[1.0], 0, 0).unwrap());

        let clone_bytes =
            ctx.get_buffer(id).unwrap().samples.capacity() * std::mem::size_of::<f32>();
        assert!(
            clone_bytes < original_bytes,
            "fixture requires a smaller clone"
        );
        assert_eq!(usage.snapshot().bytes, original_bytes + clone_bytes);
        assert_eq!(process.snapshot().bytes, original_bytes + clone_bytes);
    }

    #[test]
    fn buffer_id_overflow_is_structured_and_does_not_collide() {
        let mut ctx = AudioContext::new(1, 48_000, 2);
        ctx.next_buffer_id = i32::MAX as AudioBufferId;

        let error = ctx
            .create_empty_buffer(1, 1, 48_000)
            .expect_err("buffer ids must not silently wrap");
        assert_eq!(error.code, shared::error::ErrorCode::InputSaturated);
        assert!(ctx.buffers.is_empty());
        assert_eq!(ctx.next_buffer_id, i32::MAX as AudioBufferId);
    }

    #[test]
    fn process_fully_cleans_up_finished_nodes() {
        let mut ctx = AudioContext::new(1, 48_000, 2);

        // A buffer source wired to the destination.
        ctx.create_buffer_source(10);
        ctx.connect(10, DESTINATION_NODE_ID);

        // Per W3C, stop(when <= 0) finishes the source immediately.
        assert!(ctx.stop_source(10, 0.0));

        let mut out = vec![0.0f32; 2 * 128];
        let finished = ctx.process(&mut out);

        assert!(
            finished.contains(&10),
            "finished node id must be reported so the audio thread can unregister it"
        );
        assert!(!ctx.nodes.contains_key(&10), "removed from the node map");
        assert!(
            !ctx.node_buffers.contains_key(&10),
            "per-node output buffer must be freed"
        );
        assert!(
            !ctx.connections.iter().any(|c| c.src == 10 || c.dst == 10),
            "graph connections referencing the finished node must be dropped"
        );
    }

    #[test]
    fn remove_finished_node_purges_immediate_stop_only() {
        let mut ctx = AudioContext::new(1, 48_000, 2);

        // stop(when <= 0) finishes a buffer source immediately -> fully removed.
        ctx.create_buffer_source(20);
        ctx.connect(20, DESTINATION_NODE_ID);
        assert!(ctx.stop_source(20, 0.0));
        assert!(
            ctx.remove_finished_node(20),
            "immediate-finished node removed"
        );
        assert!(!ctx.nodes.contains_key(&20));
        assert!(!ctx.node_buffers.contains_key(&20));
        assert!(!ctx.connections.iter().any(|c| c.src == 20 || c.dst == 20));

        // A future-dated stop has NOT finished yet -> must stay reachable.
        ctx.create_buffer_source(21);
        ctx.connect(21, DESTINATION_NODE_ID);
        assert!(ctx.stop_source(21, 1000.0));
        assert!(
            !ctx.remove_finished_node(21),
            "future-dated stop must remain until it actually finishes"
        );
        assert!(ctx.nodes.contains_key(&21));
    }
}

// ── Section 7.3: zero steady-state allocation ───────────────────────────────

#[cfg(test)]
mod steady_state_allocation {
    use super::*;
    use crate::nodes::{GainNode, OscillatorNode};
    use migo_alloc_probe::{Burst, assert_no_steady_state_allocation};

    /// Section 7.3, on the audio graph's real-time path.
    ///
    /// `process` is the audio callback's work: it runs on the output thread —
    /// SCHED_FIFO on Android, per `audio_thread.rs` — once per quantum, for the
    /// life of every sound the game plays. A heap allocation here is not a
    /// throughput cost like the ones on the frame path; it is a deadline miss,
    /// heard as a dropout, because the allocator can block behind a thread that
    /// is not real-time scheduled at all.
    ///
    /// The graph is a source into a gain into the destination — the shape every
    /// `AudioBufferSourceNode`-plus-volume playback has — so the burst covers the
    /// upstream mix, a node with input, and the additive write into the output.
    #[test]
    fn steady_state_audio_quantum_never_reaches_the_heap() {
        const CHANNELS: u32 = 2;
        const QUANTUM_FRAMES: usize = 128;

        let mut ctx = AudioContext::new(1, 48_000, CHANNELS);
        let mut oscillator = OscillatorNode::new(10);
        oscillator.start(0.0);
        ctx.add_node(Box::new(oscillator));
        ctx.add_node(Box::new(GainNode::new(11)));
        ctx.connect(10, 11);
        ctx.connect(11, DESTINATION_NODE_ID);

        let mut output = vec![0.0f32; QUANTUM_FRAMES * CHANNELS as usize];

        assert_no_steady_state_allocation(
            Burst {
                path: "audio: one graph quantum on the output thread",
                // Covers the topological rebuild, the mix buffer's first resize
                // and every node's output buffer being created on first use.
                warmup: 8,
                measured: 64,
            },
            |_| {
                output.fill(0.0);
                ctx.process(&mut output)
            },
        );
    }
}
