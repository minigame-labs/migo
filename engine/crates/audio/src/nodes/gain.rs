use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

pub struct GainNode {
    id: AudioNodeId,
    gain: AudioParamTimeline,
}

impl GainNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            gain: AudioParamTimeline::new(1.0, -3.4028235e38, 3.4028235e38),
        }
    }

    pub fn set_gain(&mut self, value: f32) {
        self.gain.set_value(value);
    }

    #[allow(dead_code)]
    pub fn gain_value(&self) -> f32 {
        self.gain.value()
    }
}

impl AudioNodeProcessor for GainNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Gain
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

        let gain = self.gain.value();

        // Optimization: skip multiply if gain is 1.0
        if (gain - 1.0).abs() < f32::EPSILON {
            output[..len].copy_from_slice(&inputs[..len]);
        } else {
            for i in 0..len {
                output[i] = inputs[i] * gain;
            }
        }

        len / channels.max(1) as usize
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "gain" => Some(&mut self.gain),
            _ => None,
        }
    }
}
