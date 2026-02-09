use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// IIRFilterNode: generic IIR filter with user-provided coefficients.
///
/// Implements the transfer function:
///   H(z) = (b0 + b1*z^-1 + ... + bM*z^-M) / (1 + a1*z^-1 + ... + aN*z^-N)
///
/// Note: a0 is always 1 (normalized). The `feedback` array includes a0.
pub struct IIRFilterNode {
    id: AudioNodeId,
    /// Feedforward coefficients (numerator)
    feedforward: Vec<f64>,
    /// Feedback coefficients (denominator), a[0] is normalized to 1.0
    feedback: Vec<f64>,
    /// Per-channel input delay line
    x_history: Vec<Vec<f64>>,
    /// Per-channel output delay line
    y_history: Vec<Vec<f64>>,
    channels: u32,
}

impl IIRFilterNode {
    pub fn new(
        id: AudioNodeId,
        feedforward: Vec<f64>,
        feedback: Vec<f64>,
        channels: u32,
    ) -> Self {
        let ff_len = feedforward.len().max(1);
        let fb_len = feedback.len().max(1);

        // Normalize by a[0]
        let a0 = if feedback.is_empty() { 1.0 } else { feedback[0] };
        let inv_a0 = if a0.abs() > 1e-20 { 1.0 / a0 } else { 1.0 };

        let norm_ff: Vec<f64> = feedforward.iter().map(|&v| v * inv_a0).collect();
        let norm_fb: Vec<f64> = feedback.iter().map(|&v| v * inv_a0).collect();

        let ch = channels.max(1) as usize;
        let x_history = vec![vec![0.0; ff_len]; ch];
        let y_history = vec![vec![0.0; fb_len]; ch];

        Self {
            id,
            feedforward: norm_ff,
            feedback: norm_fb,
            x_history,
            y_history,
            channels,
        }
    }
}

impl AudioNodeProcessor for IIRFilterNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::IIRFilter
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        _sample_rate: u32,
        _channels: u32,
        _current_time: f64,
    ) -> usize {
        let len = inputs.len().min(output.len());
        if len == 0 {
            return 0;
        }

        let channels = self.channels.max(1) as usize;
        let frames = len / channels;
        let ff = &self.feedforward;
        let fb = &self.feedback;
        let ff_len = ff.len();
        let fb_len = fb.len();

        for frame in 0..frames {
            for ch in 0..channels {
                let idx = frame * channels + ch;
                let x0 = inputs[idx] as f64;

                // Shift input history
                let xh = &mut self.x_history[ch];
                for i in (1..ff_len).rev() {
                    xh[i] = xh[i - 1];
                }
                if !xh.is_empty() {
                    xh[0] = x0;
                }

                // Compute output: sum(b[i]*x[i]) - sum(a[i]*y[i]) for i>=1
                let mut y0 = 0.0;
                for i in 0..ff_len {
                    y0 += ff[i] * xh[i];
                }
                let yh = &self.y_history[ch];
                for i in 1..fb_len {
                    if i < yh.len() {
                        y0 -= fb[i] * yh[i];
                    }
                }

                // Shift output history
                let yh = &mut self.y_history[ch];
                for i in (1..fb_len).rev() {
                    yh[i] = yh[i - 1];
                }
                if !yh.is_empty() {
                    yh[0] = y0;
                }

                output[idx] = y0 as f32;
            }
        }

        frames
    }
}
