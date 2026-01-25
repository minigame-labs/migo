use shared::protocol::audio_cmd::AudioNodeId;

use super::AudioNode;

pub struct GainNode {
    #[allow(dead_code)]
    id: AudioNodeId,
    gain: f32,
}

impl GainNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self { id, gain: 1.0 }
    }

    pub fn set_gain(&mut self, value: f32) {
        self.gain = value;
    }

    #[allow(dead_code)]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Apply gain to input samples in-place
    pub fn apply(&self, samples: &mut [f32]) {
        if (self.gain - 1.0).abs() < f32::EPSILON {
            return; // No change needed
        }

        for sample in samples.iter_mut() {
            *sample *= self.gain;
        }
    }
}

impl AudioNode for GainNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn process(&mut self, output: &mut [f32], _sample_rate: u32) -> usize {
        // GainNode is a pass-through processor
        // The actual gain application happens in the mixer when processing the graph
        self.apply(output);
        output.len() / 2 // Assume stereo
    }
}
