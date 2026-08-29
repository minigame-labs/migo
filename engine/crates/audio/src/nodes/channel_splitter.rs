use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::AudioNodeProcessor;

/// ChannelSplitterNode: exposes each input channel as its own mono output port.
///
/// The node itself only has to pass its input bus through. The split happens at
/// the connection: because `output_ports()` is greater than one, the graph reads a
/// single channel -- the one the connection's output index names -- out of this
/// node's buffer and delivers it as mono.
///
/// It used to be a documented pass-through, so `createChannelSplitter()` returned
/// something that did not split and every output carried the full mix.
pub struct ChannelSplitterNode {
    id: AudioNodeId,
    number_of_outputs: u32,
}

impl ChannelSplitterNode {
    pub fn new(id: AudioNodeId, number_of_outputs: u32) -> Self {
        Self {
            id,
            number_of_outputs: number_of_outputs.clamp(1, 32),
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

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn output_ports(&self) -> u32 {
        self.number_of_outputs
    }

    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        _sample_rate: u32,
        channels: u32,
        _current_time: f64,
    ) -> usize {
        let len = inputs.len().min(output.len());
        if len > 0 {
            output[..len].copy_from_slice(&inputs[..len]);
        }
        if len < output.len() {
            // An unconnected splitter emits silence, not whatever was last here.
            output[len..].fill(0.0);
        }
        len / channels.max(1) as usize
    }
}
