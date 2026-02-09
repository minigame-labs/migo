use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// The destination node represents the final output of the audio graph.
/// It collects mixed audio from all connected source nodes.
pub struct DestinationNode {
    id: AudioNodeId,
    channels: u32,
}

impl DestinationNode {
    pub fn new(id: AudioNodeId, channels: u32) -> Self {
        Self { id, channels }
    }
}

impl AudioNodeProcessor for DestinationNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Destination
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        _sample_rate: u32,
        _channels: u32,
        _current_time: f64,
    ) -> usize {
        // Destination node: pass input to output (collected by the context)
        let len = inputs.len().min(output.len());
        if len > 0 {
            output[..len].copy_from_slice(&inputs[..len]);
        }
        len / self.channels.max(1) as usize
    }

    fn output_channels(&self) -> u32 {
        self.channels
    }
}
