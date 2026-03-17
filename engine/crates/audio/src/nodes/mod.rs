mod analyser;
mod biquad_filter;
mod buffer_source;
mod channel_merger;
mod channel_splitter;
mod constant_source;
mod delay;
mod destination;
mod dynamics_compressor;
mod gain;
mod iir_filter;
mod oscillator;
mod panner;
mod wave_shaper;

pub use analyser::AnalyserNode;
pub use biquad_filter::{BiquadFilterNode, BiquadFilterType};
pub use buffer_source::BufferSourceNode;
pub use channel_merger::ChannelMergerNode;
pub use channel_splitter::ChannelSplitterNode;
pub use constant_source::ConstantSourceNode;
pub use delay::DelayNode;
pub use destination::DestinationNode;
pub use dynamics_compressor::DynamicsCompressorNode;
pub use gain::GainNode;
pub use iir_filter::IIRFilterNode;
pub use oscillator::{OscillatorNode, OscillatorType};
pub use panner::{DistanceModel, PannerNode, PanningModel};
pub use wave_shaper::{OversampleType, WaveShaperNode};

use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

/// The type of an audio node, used for downcasting and dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioNodeType {
    BufferSource,
    Gain,
    Destination,
    Oscillator,
    BiquadFilter,
    Delay,
    Analyser,
    ChannelMerger,
    ChannelSplitter,
    ConstantSource,
    DynamicsCompressor,
    IIRFilter,
    Panner,
    WaveShaper,
    ScriptProcessor,
}

/// Trait for all audio processing nodes in the audio graph.
///
/// Each node receives mixed input from upstream connections and writes its
/// output to the provided buffer. Source nodes (BufferSource, Oscillator, etc.)
/// ignore inputs and generate audio. Processing nodes (Gain, BiquadFilter, etc.)
/// transform their input. The Destination node is the final output.
pub trait AudioNodeProcessor: Send + 'static {
    /// Get the unique node ID
    fn id(&self) -> AudioNodeId;

    /// Get the type of this node
    fn node_type(&self) -> AudioNodeType;

    /// Downcast support: return self as Any for type-specific operations
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Process audio.
    ///
    /// - `inputs`: mixed audio from all upstream connected nodes (interleaved samples).
    ///   Empty slice for source nodes with no upstream connections.
    /// - `output`: buffer to write this node's output to (interleaved samples).
    /// - `sample_rate`: the audio context's sample rate.
    /// - `channels`: the number of interleaved channels in `inputs`/`output`.
    /// - `current_time`: the current context time in seconds (for automation).
    ///
    /// Returns the number of frames written to output.
    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        sample_rate: u32,
        channels: u32,
        current_time: f64,
    ) -> usize;

    /// Check if the node has finished (for source nodes that have a finite lifetime)
    fn is_finished(&self) -> bool {
        false
    }

    /// Whether this node is a source node (generates audio, doesn't need input)
    fn is_source(&self) -> bool {
        false
    }

    /// Get the number of output channels
    fn output_channels(&self) -> u32 {
        2 // Default stereo
    }

    /// Get a named AudioParam for automation, if this node has one.
    /// Returns None if the param name is not recognized.
    fn get_param_mut(&mut self, _name: &str) -> Option<&mut AudioParamTimeline> {
        None
    }
}

/// Node connection in the audio graph
#[derive(Debug, Clone)]
pub struct NodeConnection {
    pub src: AudioNodeId,
    pub dst: AudioNodeId,
}
