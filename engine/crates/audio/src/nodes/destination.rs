use shared::protocol::audio_cmd::AudioNodeId;

use super::AudioNode;

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

impl AudioNode for DestinationNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn process(&mut self, _output: &mut [f32], _sample_rate: u32) -> usize {
        // DestinationNode doesn't process - it just receives mixed audio
        0
    }

    fn output_channels(&self) -> u32 {
        self.channels
    }
}
