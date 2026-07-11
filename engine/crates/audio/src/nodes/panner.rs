use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

/// Panning model for PannerNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanningModel {
    EqualPower,
    HRTF,
}

impl PanningModel {
    pub fn from_str(s: &str) -> Self {
        match s {
            "HRTF" => Self::HRTF,
            _ => Self::EqualPower,
        }
    }
}

/// Distance model for PannerNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceModel {
    Linear,
    Inverse,
    Exponential,
}

impl DistanceModel {
    pub fn from_str(s: &str) -> Self {
        match s {
            "linear" => Self::Linear,
            "exponential" => Self::Exponential,
            _ => Self::Inverse,
        }
    }
}

/// PannerNode: spatializes audio using equal-power panning.
///
/// Uses 3D position and orientation to compute stereo pan.
/// HRTF mode falls back to equal-power for now.
pub struct PannerNode {
    id: AudioNodeId,
    panning_model: PanningModel,
    distance_model: DistanceModel,
    position_x: AudioParamTimeline,
    position_y: AudioParamTimeline,
    position_z: AudioParamTimeline,
    orientation_x: AudioParamTimeline,
    orientation_y: AudioParamTimeline,
    orientation_z: AudioParamTimeline,
    ref_distance: f64,
    max_distance: f64,
    rolloff_factor: f64,
    cone_inner_angle: f64,
    cone_outer_angle: f64,
    cone_outer_gain: f64,
    // Cached panning gains to avoid trig recalculation when position is static
    cached_px: f64,
    cached_py: f64,
    cached_pz: f64,
    cached_gain_l: f32,
    cached_gain_r: f32,
}

impl PannerNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            panning_model: PanningModel::EqualPower,
            distance_model: DistanceModel::Inverse,
            position_x: AudioParamTimeline::new(0.0, f32::MIN, f32::MAX),
            position_y: AudioParamTimeline::new(0.0, f32::MIN, f32::MAX),
            position_z: AudioParamTimeline::new(0.0, f32::MIN, f32::MAX),
            orientation_x: AudioParamTimeline::new(1.0, f32::MIN, f32::MAX),
            orientation_y: AudioParamTimeline::new(0.0, f32::MIN, f32::MAX),
            orientation_z: AudioParamTimeline::new(0.0, f32::MIN, f32::MAX),
            ref_distance: 1.0,
            max_distance: 10000.0,
            rolloff_factor: 1.0,
            cone_inner_angle: 360.0,
            cone_outer_angle: 360.0,
            cone_outer_gain: 0.0,
            cached_px: f64::NAN, // NaN forces first computation
            cached_py: f64::NAN,
            cached_pz: f64::NAN,
            cached_gain_l: std::f32::consts::FRAC_1_SQRT_2,
            cached_gain_r: std::f32::consts::FRAC_1_SQRT_2,
        }
    }

    pub fn set_panning_model(&mut self, model: PanningModel) {
        self.panning_model = model;
    }

    pub fn set_distance_model(&mut self, model: DistanceModel) {
        self.distance_model = model;
    }

    pub fn set_ref_distance(&mut self, v: f64) {
        self.ref_distance = v.max(0.0);
    }

    pub fn set_max_distance(&mut self, v: f64) {
        self.max_distance = v.max(0.0);
    }

    pub fn set_rolloff_factor(&mut self, v: f64) {
        self.rolloff_factor = v.max(0.0);
    }

    pub fn set_cone_inner_angle(&mut self, v: f64) {
        self.cone_inner_angle = v;
    }

    pub fn set_cone_outer_angle(&mut self, v: f64) {
        self.cone_outer_angle = v;
    }

    pub fn set_cone_outer_gain(&mut self, v: f64) {
        self.cone_outer_gain = v.clamp(0.0, 1.0);
    }

    /// Compute distance attenuation based on distance model
    fn compute_distance_gain(&self, distance: f64) -> f64 {
        match self.distance_model {
            DistanceModel::Linear => {
                let d = distance.clamp(self.ref_distance, self.max_distance);
                if self.max_distance == self.ref_distance {
                    1.0
                } else {
                    1.0 - self.rolloff_factor * (d - self.ref_distance)
                        / (self.max_distance - self.ref_distance)
                }
            }
            DistanceModel::Inverse => {
                let d = distance.max(self.ref_distance);
                if self.ref_distance == 0.0 {
                    1.0
                } else {
                    self.ref_distance
                        / (self.ref_distance + self.rolloff_factor * (d - self.ref_distance))
                }
            }
            DistanceModel::Exponential => {
                let d = distance.max(self.ref_distance);
                if self.ref_distance == 0.0 {
                    1.0
                } else {
                    (d / self.ref_distance).powf(-self.rolloff_factor)
                }
            }
        }
    }
}

impl AudioNodeProcessor for PannerNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Panner
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
        current_time: f64,
    ) -> usize {
        let ch = channels.max(1) as usize;
        let len = inputs.len().min(output.len());
        if len == 0 {
            return 0;
        }

        // Get source position
        let px = self.position_x.compute_value(current_time) as f64;
        let py = self.position_y.compute_value(current_time) as f64;
        let pz = self.position_z.compute_value(current_time) as f64;

        // Recompute gains only if position changed (skip expensive trig otherwise)
        if px != self.cached_px || py != self.cached_py || pz != self.cached_pz {
            self.cached_px = px;
            self.cached_py = py;
            self.cached_pz = pz;

            // Listener at origin (simplified — will use AudioListener later)
            let distance = (px * px + py * py + pz * pz).sqrt();
            let distance_gain = self.compute_distance_gain(distance);

            // Equal-power panning based on azimuth (x position)
            let azimuth = if distance > 0.0001 {
                (px / distance).asin()
            } else {
                0.0
            };

            // Map azimuth [-PI/2, PI/2] to pan [0, 1] where 0=left, 1=right
            let pan = (azimuth / std::f64::consts::FRAC_PI_2 * 0.5 + 0.5).clamp(0.0, 1.0);

            // Equal-power panning
            let angle = pan * std::f64::consts::FRAC_PI_2;
            self.cached_gain_l = (angle.cos() * distance_gain) as f32;
            self.cached_gain_r = (angle.sin() * distance_gain) as f32;
        }

        let gain_l = self.cached_gain_l;
        let gain_r = self.cached_gain_r;

        let frames = len / ch;
        for frame in 0..frames {
            let base = frame * ch;
            // Mix all input channels to mono
            let mut mono = 0.0f32;
            for c in 0..ch {
                mono += inputs[base + c];
            }
            mono /= ch as f32;

            // Write panned output: ch0 = left, ch1 = right, rest = 0
            output[base] = mono * gain_l;
            if ch >= 2 {
                output[base + 1] = mono * gain_r;
            }
            for c in 2..ch {
                output[base + c] = 0.0;
            }
        }

        frames
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "positionX" => Some(&mut self.position_x),
            "positionY" => Some(&mut self.position_y),
            "positionZ" => Some(&mut self.position_z),
            "orientationX" => Some(&mut self.orientation_x),
            "orientationY" => Some(&mut self.orientation_y),
            "orientationZ" => Some(&mut self.orientation_z),
            _ => None,
        }
    }
}
