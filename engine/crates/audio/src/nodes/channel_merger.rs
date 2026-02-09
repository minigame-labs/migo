use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// ChannelMergerNode: merges multiple mono inputs into a single multi-channel output.
///
/// For a stereo output (most common case), input 0 goes to left channel
/// and input 1 goes to right channel. Since our audio graph currently uses
/// a single mixed input buffer, this node simply passes through the input.
pub struct ChannelMergerNode {
    id: AudioNodeId,
    number_of_inputs: u32,
}

impl ChannelMergerNode {
    pub fn new(id: AudioNodeId, number_of_inputs: u32) -> Self {
        Self {
            id,
            number_of_inputs: number_of_inputs.max(1).min(32),
        }
    }

    #[allow(dead_code)]
    pub fn number_of_inputs(&self) -> u32 {
        self.number_of_inputs
    }
}

impl AudioNodeProcessor for ChannelMergerNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::ChannelMerger
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
        // Simplified: pass through mixed input
        let len = inputs.len().min(output.len());
        if len > 0 {
            output[..len].copy_from_slice(&inputs[..len]);
        }
        len / channels.max(1) as usize
    }

    fn output_channels(&self) -> u32 {
        self.number_of_inputs
    }
}
