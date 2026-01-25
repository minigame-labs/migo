#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use shared::protocol::audio_cmd::{AudioBufferId, AudioContextId, AudioContextState, AudioNodeId};

use crate::decoder::DecodedAudio;
use crate::mixer::Mixer;
use crate::nodes::{BufferSourceNode, DestinationNode, GainNode, NodeConnection};

/// Special node ID for the destination node
pub const DESTINATION_NODE_ID: AudioNodeId = 0;

/// Pre-computed routing info for a source node
#[derive(Clone)]
struct SourceRoute {
    source_id: AudioNodeId,
    gain_id: Option<AudioNodeId>,
}

/// An AudioContext manages audio resources and the audio graph
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

    // Nodes
    source_nodes: HashMap<AudioNodeId, BufferSourceNode>,
    gain_nodes: HashMap<AudioNodeId, GainNode>,
    destination: DestinationNode,
    next_node_id: AudioNodeId,

    // Graph connections
    connections: Vec<NodeConnection>,
    // Pre-computed routes (updated when connections change)
    routes: Vec<SourceRoute>,
    routes_dirty: bool,

    // Mixer
    mixer: Mixer,

    // Processing buffers (pre-allocated)
    process_buffer: Vec<f32>,
}

impl AudioContext {
    pub fn new(id: AudioContextId, sample_rate: u32, channels: u32) -> Self {
        Self {
            id,
            state: AudioContextState::Running,
            sample_rate,
            channels,
            start_time: Instant::now(),
            buffers: HashMap::new(),
            next_buffer_id: 1,
            source_nodes: HashMap::new(),
            gain_nodes: HashMap::new(),
            destination: DestinationNode::new(DESTINATION_NODE_ID, channels),
            next_node_id: 1, // 0 is reserved for destination
            connections: Vec::new(),
            routes: Vec::new(),
            routes_dirty: true,
            mixer: Mixer::new(channels),
            process_buffer: Vec::new(),
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
        self.source_nodes.clear();
        self.gain_nodes.clear();
        self.connections.clear();
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

    // ==================== Node Management ====================

    /// Create a buffer source node with JS-provided node_id
    pub fn create_buffer_source(&mut self, node_id: AudioNodeId) {
        tracing::trace!("create_buffer_source: node_id={}", node_id);
        self.source_nodes
            .insert(node_id, BufferSourceNode::new(node_id));
    }

    /// Create a gain node with JS-provided node_id
    pub fn create_gain(&mut self, node_id: AudioNodeId) {
        tracing::trace!("create_gain: node_id={}", node_id);
        self.gain_nodes.insert(node_id, GainNode::new(node_id));
    }

    pub fn set_buffer(&mut self, node_id: AudioNodeId, buffer_id: AudioBufferId) -> bool {
        tracing::trace!("set_buffer: node_id={}, buffer_id={}", node_id, buffer_id);
        if let Some(node) = self.source_nodes.get_mut(&node_id) {
            if let Some(buffer) = self.buffers.get(&buffer_id) {
                tracing::trace!(
                    "set_buffer: found node and buffer, samples={}, channels={}, sample_rate={}",
                    buffer.samples.len(),
                    buffer.channels,
                    buffer.sample_rate
                );
                node.set_buffer(buffer.clone());
                return true;
            } else {
                tracing::warn!("set_buffer: buffer {} not found", buffer_id);
            }
        } else {
            tracing::warn!(
                "set_buffer: source node {} not found, available: {:?}",
                node_id,
                self.source_nodes.keys().collect::<Vec<_>>()
            );
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
        if let Some(node) = self.source_nodes.get_mut(&node_id) {
            node.start(when, offset, duration, current_time);
            tracing::trace!("start_source: node started");
            return true;
        }
        tracing::warn!("start_source: node {} not found", node_id);
        false
    }

    pub fn stop_source(&mut self, node_id: AudioNodeId, when: f64) -> bool {
        if let Some(node) = self.source_nodes.get_mut(&node_id) {
            node.stop(when);
            return true;
        }
        false
    }

    pub fn set_loop(&mut self, node_id: AudioNodeId, enabled: bool, start: f64, end: f64) -> bool {
        if let Some(node) = self.source_nodes.get_mut(&node_id) {
            node.set_loop(enabled, start, end);
            return true;
        }
        false
    }

    pub fn set_playback_rate(&mut self, node_id: AudioNodeId, rate: f32) -> bool {
        if let Some(node) = self.source_nodes.get_mut(&node_id) {
            node.set_playback_rate(rate);
            return true;
        }
        false
    }

    pub fn set_gain(&mut self, node_id: AudioNodeId, value: f32) -> bool {
        if let Some(node) = self.gain_nodes.get_mut(&node_id) {
            node.set_gain(value);
            return true;
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
            self.routes_dirty = true;
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
            self.routes_dirty = true;
        }
    }

    /// Rebuild the pre-computed routes from the connection graph
    fn rebuild_routes(&mut self) {
        tracing::trace!("rebuild_routes: connections={:?}", self.connections);
        self.routes.clear();

        // Find all nodes connected to destination
        for conn in &self.connections {
            if conn.dst == DESTINATION_NODE_ID {
                // Check if it's a source node (direct connection)
                if self.source_nodes.contains_key(&conn.src) {
                    tracing::trace!("rebuild_routes: direct route from source {}", conn.src);
                    self.routes.push(SourceRoute {
                        source_id: conn.src,
                        gain_id: None,
                    });
                }
                // Check if it's a gain node
                else if self.gain_nodes.contains_key(&conn.src) {
                    let gain_id = conn.src;
                    // Find sources connected to this gain node
                    for inner_conn in &self.connections {
                        if inner_conn.dst == gain_id
                            && self.source_nodes.contains_key(&inner_conn.src)
                        {
                            tracing::trace!(
                                "rebuild_routes: route from source {} via gain {}",
                                inner_conn.src,
                                gain_id
                            );
                            self.routes.push(SourceRoute {
                                source_id: inner_conn.src,
                                gain_id: Some(gain_id),
                            });
                        }
                    }
                }
            }
        }
        tracing::trace!("rebuild_routes: total routes={}", self.routes.len());

        self.routes_dirty = false;
    }

    // ==================== Processing ====================

    /// Process audio and fill the output buffer
    pub fn process(&mut self, output: &mut [f32]) {
        static PROCESS_LOG_COUNTER: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);

        if self.state != AudioContextState::Running {
            output.fill(0.0);
            return;
        }

        // Rebuild routes if graph changed
        if self.routes_dirty {
            self.rebuild_routes();
        }

        let channels = self.channels as usize;
        let buffer_size = output.len();

        // Ensure process buffer is large enough
        if self.process_buffer.len() < buffer_size {
            self.process_buffer.resize(buffer_size, 0.0);
        }

        // Clear output
        output.fill(0.0);

        // Clone routes to avoid borrow issues (small vec, cheap clone)
        let routes = self.routes.clone();

        // Log every 100 calls
        let counter = PROCESS_LOG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let should_log = counter % 100 == 0;

        if should_log {
            tracing::trace!(
                "process: routes={}, source_nodes={}",
                routes.len(),
                self.source_nodes.len()
            );
        }

        // Process each route
        for route in &routes {
            let source = match self.source_nodes.get_mut(&route.source_id) {
                Some(s) => s,
                None => continue,
            };

            let buffer = &mut self.process_buffer[..buffer_size];
            buffer.fill(0.0);
            let frames_written = source.process_with_channels(buffer, channels as u32);

            if should_log && frames_written > 0 {
                tracing::trace!(
                    "process: source {} wrote {} frames",
                    route.source_id,
                    frames_written
                );
            }

            // Apply gain if present
            if let Some(gain_id) = route.gain_id {
                if let Some(gain) = self.gain_nodes.get(&gain_id) {
                    gain.apply(buffer);
                }
            }

            // Mix into output (SIMD-friendly loop)
            for i in 0..buffer_size {
                output[i] += buffer[i];
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

        // Clean up finished source nodes (and mark routes dirty)
        let old_count = self.source_nodes.len();
        self.source_nodes.retain(|_, node| !node.is_finished());
        if self.source_nodes.len() != old_count {
            self.routes_dirty = true;
        }
    }
}
