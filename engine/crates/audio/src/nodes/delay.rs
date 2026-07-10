use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

/// DelayNode: circular buffer delay line.
///
/// Delays the input signal by a specified amount of time.
/// The `delayTime` parameter can be automated for effects like flanging.
pub struct DelayNode {
    id: AudioNodeId,
    delay_time: AudioParamTimeline,
    /// Circular buffer for delay (interleaved stereo)
    buffer: Vec<f32>,
    /// Write position in the circular buffer (in samples, not frames)
    write_pos: usize,
    /// Maximum delay in seconds (set at creation time)
    max_delay: f32,
    channels: u32,
}

impl DelayNode {
    pub fn new(id: AudioNodeId, max_delay_time: f32, sample_rate: u32, channels: u32) -> Self {
        // Per-node delay-buffer memory cap. Web Audio allows maxDelayTime up to
        // 180s, which at 48kHz stereo f32 is ~68MB for a single node; cap the
        // allocation and shrink max_delay to fit the budget.
        const MAX_DELAY_BYTES: usize = 16 * 1024 * 1024;
        let ch = channels.max(1) as usize;
        let requested = max_delay_time.max(0.001).min(179.0); // Web Audio spec max
        let bytes_per_sec = sample_rate as f64 * ch as f64 * std::mem::size_of::<f32>() as f64;
        let budget_secs = (MAX_DELAY_BYTES as f64 / bytes_per_sec.max(1.0)) as f32;
        let max_delay = requested.min(budget_secs);
        if max_delay < requested {
            tracing::warn!(
                "DelayNode {}: maxDelayTime {:.1}s exceeds the {}MB per-node budget; clamped to {:.1}s",
                id,
                requested,
                MAX_DELAY_BYTES / (1024 * 1024),
                max_delay
            );
        }
        let buffer_size = (max_delay as f64 * sample_rate as f64) as usize * ch;
        Self {
            id,
            delay_time: AudioParamTimeline::new(0.0, 0.0, max_delay),
            buffer: vec![0.0; buffer_size],
            write_pos: 0,
            max_delay,
            channels,
        }
    }
}

impl AudioNodeProcessor for DelayNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Delay
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn process(
        &mut self,
        inputs: &[f32],
        output: &mut [f32],
        sample_rate: u32,
        _channels: u32,
        current_time: f64,
    ) -> usize {
        let len = inputs.len().min(output.len());
        if len == 0 || self.buffer.is_empty() {
            return 0;
        }

        let channels = self.channels as usize;
        let frames = len / channels;
        let buf_len = self.buffer.len();

        let delay_secs = self.delay_time.compute_value(current_time).max(0.0).min(self.max_delay);
        let delay_samples = (delay_secs as f64 * sample_rate as f64) as usize * channels;

        for frame in 0..frames {
            for ch in 0..channels {
                let idx = frame * channels + ch;

                // Write input to circular buffer
                self.buffer[self.write_pos] = inputs[idx];

                // Read from circular buffer with delay
                let read_pos = if self.write_pos >= delay_samples {
                    self.write_pos - delay_samples
                } else {
                    buf_len + self.write_pos - delay_samples
                };

                output[idx] = self.buffer[read_pos % buf_len];

                self.write_pos = (self.write_pos + 1) % buf_len;
            }
        }

        frames
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "delayTime" => Some(&mut self.delay_time),
            _ => None,
        }
    }
}
