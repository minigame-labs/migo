use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// ChannelSplitterNode: splits a multi-channel input into separate mono outputs.
///
/// Since our graph uses a single mixed output buffer per node, this is simplified
/// to pass through the input unchanged. Full per-output routing would require
/// multi-output support in the graph.
pub struct ChannelSplitterNode {
    id: AudioNodeId,
    number_of_outputs: u32,
}

impl ChannelSplitterNode {
    pub fn new(id: AudioNodeId, number_of_outputs: u32) -> Self {
        Self {
            id,
            number_of_outputs: number_of_outputs.max(1).min(32),
        }
    }

    #[allow(dead_code)]
    pub fn number_of_outputs(&self) -> u32 {
        self.number_of_outputs
    }
}

impl AudioNodeProcessor for ChannelSplitterNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::ChannelSplitter
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        _sample_rate: u32,
        channels: u32,
        _current_time: f64,
    ) -> usize {
        // Simplified: pass through
        let len = inputs.len().min(output.len());
        if len > 0 {
            output[..len].copy_from_slice(&inputs[..len]);
        }
        len / channels.max(1) as usize
    }
}
