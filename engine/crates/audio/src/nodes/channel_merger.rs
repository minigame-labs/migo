use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::AudioNodeProcessor;

/// ChannelMergerNode: gathers one mono input per port into one multi-channel bus.
///
/// The node itself only has to pass its already-gathered input bus through. The
/// merge happens at the connection: because `input_ports()` is greater than one,
/// the graph writes each incoming connection into the single channel its input
/// index names rather than mixing it across the whole bus.
///
/// It used to be a documented pass-through, so `createChannelMerger()` summed
/// every input into every channel instead of placing them side by side.
pub struct ChannelMergerNode {
    id: AudioNodeId,
    number_of_inputs: u32,
}

impl ChannelMergerNode {
    pub fn new(id: AudioNodeId, number_of_inputs: u32) -> Self {
        Self {
            id,
            number_of_inputs: number_of_inputs.clamp(1, 32),
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

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn input_ports(&self) -> u32 {
        self.number_of_inputs
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
            output[len..].fill(0.0);
        }
        len / channels.max(1) as usize
    }

    fn output_channels(&self) -> u32 {
        self.number_of_inputs
    }
}
