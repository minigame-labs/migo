use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

/// ConstantSourceNode: outputs a constant value from its `offset` AudioParam.
///
/// Useful for automation — the offset can be automated and connected to
/// other nodes' AudioParam inputs for complex modulation.
pub struct ConstantSourceNode {
    id: AudioNodeId,
    offset: AudioParamTimeline,
    started: bool,
    stopped: bool,
    start_when: f64,
    stop_when: f64,
}

impl ConstantSourceNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            offset: AudioParamTimeline::new(1.0, f32::MIN, f32::MAX),
            started: false,
            stopped: false,
            start_when: 0.0,
            stop_when: f64::INFINITY,
        }
    }

    pub fn start(&mut self, when: f64) {
        self.started = true;
        self.start_when = when;
    }

    pub fn stop(&mut self, when: f64) {
        self.stop_when = when;
    }
}

impl AudioNodeProcessor for ConstantSourceNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::ConstantSource
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
        _sample_rate: u32,
        current_time: f64,
    ) -> usize {
        if !self.started || current_time < self.start_when {
            return 0;
        }

        if current_time >= self.stop_when {
            self.stopped = true;
            return 0;
        }

        let value = self.offset.value();
        let channels = 2usize;
        let frames = output.len() / channels;

        for i in 0..output.len().min(frames * channels) {
            output[i] = value;
        }

        frames
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "offset" => Some(&mut self.offset),
            _ => None,
        }
    }
}
