#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use shared::protocol::audio_cmd::{AudioBufferId, AudioContextId, AudioContextState, AudioNodeId};

use crate::decoder::DecodedAudio;
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

    // Timing
    start_time: Instant,

    // Resources
    buffers: HashMap<AudioBufferId, Arc<DecodedAudio>>,
    next_buffer_id: AudioBufferId,

    // Generic node storage
    nodes: HashMap<AudioNodeId, Box<dyn AudioNodeProcessor>>,

    // Graph connections
    connections: Vec<NodeConnection>,

    // Topologically sorted processing order (cached, invalidated on graph change)
    processing_order: Vec<AudioNodeId>,
    graph_dirty: bool,

    // Processing buffers: per-node output buffers keyed by node ID
    node_buffers: HashMap<AudioNodeId, Vec<f32>>,
    // Scratch buffer for mixing multiple inputs
    mix_buffer: Vec<f32>,
}

impl AudioContext {
    /// Default capacity for audio resources.
    const DEFAULT_BUFFER_CAPACITY: usize = 8;
    const DEFAULT_NODE_CAPACITY: usize = 16;

    pub fn new(id: AudioContextId, sample_rate: u32, channels: u32) -> Self {
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
            start_time: Instant::now(),
            buffers: HashMap::with_capacity(Self::DEFAULT_BUFFER_CAPACITY),
            next_buffer_id: 1,
            nodes,
            connections: Vec::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            processing_order: Vec::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            graph_dirty: true,
            node_buffers: HashMap::with_capacity(Self::DEFAULT_NODE_CAPACITY),
            mix_buffer: Vec::new(),
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

    /// Get the current time in seconds since context creation
    pub fn current_time(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
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
        self.node_buffers.clear();
    }

    // ==================== Buffer Management ====================

    pub fn add_buffer(&mut self, audio: DecodedAudio) -> AudioBufferId {
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;
        self.buffers.insert(id, Arc::new(audio));
        id
    }

    pub fn get_buffer(&self, id: AudioBufferId) -> Option<Arc<DecodedAudio>> {
        self.buffers.get(&id).cloned()
    }

    pub fn remove_buffer(&mut self, id: AudioBufferId) -> bool {
        self.buffers.remove(&id).is_some()
    }

    /// Create an empty buffer with the given parameters
    pub fn create_empty_buffer(
        &mut self,
        channels: u32,
        length: u32,
        sample_rate: u32,
    ) -> AudioBufferId {
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let samples = vec![0.0f32; (length * channels) as usize];
        let audio = DecodedAudio {
            samples,
            sample_rate,
            channels,
        };
        self.buffers.insert(id, Arc::new(audio));
        id
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

    /// Copy data into a specific channel of a buffer (copy-on-write via Arc::make_mut).
    pub fn copy_to_channel(
        &mut self,
        buffer_id: AudioBufferId,
        data: &[f32],
        channel: u32,
        start_frame: u32,
    ) -> bool {
        let buffer = match self.buffers.get_mut(&buffer_id) {
            Some(b) => b,
            None => return false,
        };

        if channel >= buffer.channels {
            return false;
        }

        let channels = buffer.channels as usize;
        let frame_count = buffer.frame_count();
        let start = start_frame as usize;
        let ch = channel as usize;

        // Clamp copy length to buffer bounds
        let copy_len = data.len().min(frame_count.saturating_sub(start));
        if copy_len == 0 {
            return true; // Nothing to copy, but not an error
        }

        // Arc::make_mut clones the inner data only if there are other references
        let audio = Arc::make_mut(buffer);

        for i in 0..copy_len {
            let sample_idx = (start + i) * channels + ch;
            if sample_idx < audio.samples.len() {
                audio.samples[sample_idx] = data[i];
            }
        }

        true
    }

    // ==================== Node Management ====================

    /// Create a buffer source node with JS-provided node_id
    pub fn create_buffer_source(&mut self, node_id: AudioNodeId) {
        tracing::trace!("create_buffer_source: node_id={}", node_id);
        self.nodes
            .insert(node_id, Box::new(BufferSourceNode::new(node_id)));
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

    pub fn set_buffer(&mut self, node_id: AudioNodeId, buffer_id: AudioBufferId) -> bool {
        tracing::trace!("set_buffer: node_id={}, buffer_id={}", node_id, buffer_id);
        let buffer = match self.buffers.get(&buffer_id) {
            Some(b) => b.clone(),
            None => {
                tracing::warn!("set_buffer: buffer {} not found", buffer_id);
                return false;
            }
        };

        if let Some(node) = self.nodes.get_mut(&node_id) {
            let any = node.as_any_mut();
            if let Some(source) = any.downcast_mut::<BufferSourceNode>() {
                tracing::trace!(
                    "set_buffer: found node and buffer, samples={}, channels={}, sample_rate={}",
                    buffer.samples.len(),
                    buffer.channels,
                    buffer.sample_rate
                );
                source.set_buffer(buffer);
                return true;
            }
        }
        tracing::warn!("set_buffer: source node {} not found", node_id);
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
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_value(value);
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
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_value_at_time(value, time);
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
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.linear_ramp_to_value_at_time(value, end_time);
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
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.exponential_ramp_to_value_at_time(value, end_time);
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
        if let Some(node) = self.nodes.get_mut(&node_id) {
            if let Some(param) = node.get_param_mut(param_name) {
                param.set_target_at_time(target, start_time, time_constant);
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

    /// Rebuild the processing order using topological sort (Kahn's algorithm).
    ///
    /// This produces a valid processing order where every node is processed
    /// after all of its input sources, supporting arbitrary graph topologies
    /// like: source → filter → gain → destination.
    fn rebuild_processing_order(&mut self) {
        tracing::trace!(
            "rebuild_processing_order: connections={:?}",
            self.connections
        );
        self.processing_order.clear();

        // Build adjacency list and in-degree count
        let mut in_degree: HashMap<AudioNodeId, usize> = HashMap::new();
        let mut adj: HashMap<AudioNodeId, Vec<AudioNodeId>> = HashMap::new();

        // Initialize all nodes with in-degree 0
        for &node_id in self.nodes.keys() {
            in_degree.insert(node_id, 0);
            adj.entry(node_id).or_default();
        }

        // Count in-degrees from connections
        for conn in &self.connections {
            if self.nodes.contains_key(&conn.src) && self.nodes.contains_key(&conn.dst) {
                *in_degree.entry(conn.dst).or_default() += 1;
                adj.entry(conn.src).or_default().push(conn.dst);
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

    /// Process audio and fill the output buffer.
    ///
    /// Uses topological sort to process nodes in dependency order.
    /// Each node receives its upstream mixed inputs and writes to its own buffer.
    /// The destination node's output is copied to the final output.
    pub fn process(&mut self, output: &mut [f32]) {
        static PROCESS_LOG_COUNTER: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);

        if self.state != AudioContextState::Running {
            output.fill(0.0);
            return;
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

        // Log every 100 calls
        let counter = PROCESS_LOG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let should_log = counter % 100 == 0;

        if should_log {
            tracing::trace!(
                "process: order={:?}, nodes={}",
                self.processing_order,
                self.nodes.len()
            );
        }

        // Clone processing order to avoid borrow issues
        let order = self.processing_order.clone();

        // Process each node in topological order
        for &node_id in &order {
            // First, gather mixed input from all upstream connections
            self.mix_buffer[..buffer_size].fill(0.0);
            let mut has_input = false;

            // Find all connections where dst == node_id, mix their src outputs
            for conn in &self.connections {
                if conn.dst == node_id {
                    if let Some(src_buf) = self.node_buffers.get(&conn.src) {
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

                let frames = node.process(
                    input,
                    &mut node_buf[..buffer_size],
                    sample_rate,
                    self.channels,
                    current_time,
                );

                if should_log && frames > 0 {
                    tracing::trace!(
                        "process: node {} ({:?}) wrote {} frames",
                        node_id,
                        node.node_type(),
                        frames
                    );
                }
            }
        }

        // Copy destination node's output to final output
        if let Some(dest_buf) = self.node_buffers.get(&DESTINATION_NODE_ID) {
            let len = dest_buf.len().min(buffer_size);
            output[..len].copy_from_slice(&dest_buf[..len]);
        } else {
            output.fill(0.0);
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

        // Clean up finished source nodes
        let old_count = self.nodes.len();
        self.nodes.retain(|_, node| !node.is_finished());
        if self.nodes.len() != old_count {
            self.graph_dirty = true;
        }
    }
}
