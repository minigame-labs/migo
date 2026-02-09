use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

/// Waveform type for OscillatorNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OscillatorType {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

impl OscillatorType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "square" => Self::Square,
            "sawtooth" => Self::Sawtooth,
            "triangle" => Self::Triangle,
            _ => Self::Sine,
        }
    }
}

pub struct OscillatorNode {
    id: AudioNodeId,
    frequency: AudioParamTimeline,
    detune: AudioParamTimeline,
    osc_type: OscillatorType,
    phase: f64,
    started: bool,
    stopped: bool,
    start_when: f64,
    stop_when: f64,
}

impl OscillatorNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            // Default frequency: 440 Hz (A4)
            frequency: AudioParamTimeline::new(440.0, -22050.0, 22050.0),
            // Default detune: 0 cents
            detune: AudioParamTimeline::new(0.0, -153600.0, 153600.0),
            osc_type: OscillatorType::Sine,
            phase: 0.0,
            started: false,
            stopped: false,
            start_when: 0.0,
            stop_when: f64::INFINITY,
        }
    }

    pub fn set_type(&mut self, t: OscillatorType) {
        self.osc_type = t;
    }

    pub fn start(&mut self, when: f64) {
        self.started = true;
        self.start_when = when;
    }

    pub fn stop(&mut self, when: f64) {
        self.stop_when = when;
    }

    /// Generate a single sample for the given phase [0, 1)
    #[inline]
    fn generate_sample(osc_type: OscillatorType, phase: f64) -> f32 {
        match osc_type {
            OscillatorType::Sine => {
                (phase * std::f64::consts::TAU).sin() as f32
            }
            OscillatorType::Square => {
                if phase < 0.5 { 1.0 } else { -1.0 }
            }
            OscillatorType::Sawtooth => {
                (2.0 * phase - 1.0) as f32
            }
            OscillatorType::Triangle => {
                let v = 2.0 * phase;
                if v < 1.0 {
                    (2.0 * v - 1.0) as f32
                } else {
                    (3.0 - 2.0 * v) as f32
                }
            }
        }
    }
}

impl AudioNodeProcessor for OscillatorNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Oscillator
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_source(&self) -> bool {
        true
    }

    fn is_finished(&self) -> bool {
        self.stopped
    }

    fn process(
        &mut self,
        _inputs: &[f32],
        output: &mut [f32],
        sample_rate: u32,
        channels: u32,
        current_time: f64,
    ) -> usize {
        if !self.started || current_time < self.start_when {
            return 0;
        }

        if current_time >= self.stop_when {
            self.stopped = true;
            return 0;
        }

        let channels = channels.max(1);
        let frames = output.len() / channels as usize;
        if frames == 0 {
            return 0;
        }

        let sr = sample_rate as f64;
        let freq = self.frequency.value() as f64;
        let detune = self.detune.value() as f64;

        // Compute actual frequency with detune (cents to ratio)
        let computed_freq = freq * (2.0f64).powf(detune / 1200.0);
        let phase_inc = computed_freq / sr;

        for frame in 0..frames {
            let sample = Self::generate_sample(self.osc_type, self.phase);

            // Write to all channels
            for ch in 0..channels as usize {
                output[frame * channels as usize + ch] = sample;
            }

            self.phase += phase_inc;
            // Keep phase in [0, 1)
            self.phase -= self.phase.floor();
        }

        frames
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "frequency" => Some(&mut self.frequency),
            "detune" => Some(&mut self.detune),
            _ => None,
        }
    }
}
