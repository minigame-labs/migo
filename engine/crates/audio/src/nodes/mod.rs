mod buffer_source;
mod destination;
mod gain;

pub use buffer_source::BufferSourceNode;
pub use destination::DestinationNode;
pub use gain::GainNode;

use shared::protocol::audio_cmd::AudioNodeId;

#[allow(dead_code)]
/// Trait for all audio nodes
pub trait AudioNode: Send {
    /// Get the node ID
    fn id(&self) -> AudioNodeId;

    /// Process audio samples
    /// Returns the number of frames written to output
    fn process(&mut self, output: &mut [f32], sample_rate: u32) -> usize;

    /// Check if the node has finished (for source nodes)
    fn is_finished(&self) -> bool {
        false
    }

    /// Get the number of output channels
    fn output_channels(&self) -> u32 {
        2 // Default stereo
    }
}

/// Node connection in the audio graph
#[derive(Debug, Clone)]
pub struct NodeConnection {
    pub src: AudioNodeId,
    pub dst: AudioNodeId,
}
