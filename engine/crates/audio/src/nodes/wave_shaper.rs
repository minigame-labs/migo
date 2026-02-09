use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// Oversample mode for WaveShaperNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OversampleType {
    None,
    TwoX,
    FourX,
}

impl OversampleType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "2x" => Self::TwoX,
            "4x" => Self::FourX,
            _ => Self::None,
        }
    }
}

/// WaveShaperNode: applies a non-linear distortion using a transfer function.
///
/// The `curve` is a Float32Array lookup table. Input values in [-1, 1] are
/// mapped to indices in the curve array with linear interpolation.
pub struct WaveShaperNode {
    id: AudioNodeId,
    curve: Option<Vec<f32>>,
    oversample: OversampleType,
}

impl WaveShaperNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            curve: None,
            oversample: OversampleType::None,
        }
    }

    pub fn set_curve(&mut self, curve: Option<Vec<f32>>) {
        self.curve = curve;
    }

    pub fn set_oversample(&mut self, oversample: OversampleType) {
        self.oversample = oversample;
    }

    /// Apply waveshaping transfer function using linear interpolation
    #[inline]
    fn shape(curve: &[f32], input: f32) -> f32 {
        if curve.is_empty() {
            return input;
        }

        let len = curve.len();
        if len == 1 {
            return curve[0];
        }

        // Map input [-1, 1] to curve index [0, len-1]
        let clamped = input.clamp(-1.0, 1.0);
        let normalized = (clamped + 1.0) * 0.5; // [0, 1]
        let index_f = normalized * (len - 1) as f32;

        let index_low = (index_f as usize).min(len - 2);
        let index_high = index_low + 1;
        let frac = index_f - index_low as f32;

        // Linear interpolation
        curve[index_low] * (1.0 - frac) + curve[index_high] * frac
    }
}

impl AudioNodeProcessor for WaveShaperNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::WaveShaper
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
        let len = inputs.len().min(output.len());
        if len == 0 {
            return 0;
        }

        match &self.curve {
            Some(curve) => {
                // Apply waveshaping (no oversampling for now — oversample
                // modes will be implemented later if needed for quality)
                for i in 0..len {
                    output[i] = Self::shape(curve, inputs[i]);
                }
            }
            None => {
                // No curve: pass through
                output[..len].copy_from_slice(&inputs[..len]);
            }
        }

        len / channels.max(1) as usize
    }
}
