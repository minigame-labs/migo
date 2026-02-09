use std::any::Any;

use shared::protocol::audio_cmd::AudioNodeId;

use super::{AudioNodeProcessor, AudioNodeType};

/// AnalyserNode: provides real-time frequency and time-domain analysis.
///
/// Passes audio through unchanged while capturing samples for FFT analysis.
/// Uses a simple DFT for now; can be upgraded to use `rustfft` for performance.
pub struct AnalyserNode {
    id: AudioNodeId,
    fft_size: usize,
    min_decibels: f32,
    max_decibels: f32,
    smoothing_time_constant: f32,
    /// Circular time-domain buffer (mono, downmixed from input)
    time_domain_buffer: Vec<f32>,
    write_pos: usize,
    channels: u32,
}

impl AnalyserNode {
    pub fn new(id: AudioNodeId, channels: u32) -> Self {
        let fft_size = 2048; // Default per spec
        Self {
            id,
            fft_size,
            min_decibels: -100.0,
            max_decibels: -30.0,
            smoothing_time_constant: 0.8,
            time_domain_buffer: vec![0.0; fft_size],
            write_pos: 0,
            channels,
        }
    }

    pub fn set_fft_size(&mut self, size: usize) {
        // Must be power of 2, between 32 and 32768
        let size = size.next_power_of_two().max(32).min(32768);
        if size != self.fft_size {
            self.fft_size = size;
            self.time_domain_buffer.resize(size, 0.0);
            self.write_pos = 0;
        }
    }

    pub fn set_min_decibels(&mut self, v: f32) {
        self.min_decibels = v;
    }

    pub fn set_max_decibels(&mut self, v: f32) {
        self.max_decibels = v;
    }

    pub fn set_smoothing_time_constant(&mut self, v: f32) {
        self.smoothing_time_constant = v.clamp(0.0, 1.0);
    }

    /// Get the current time domain data as byte values (0-255)
    pub fn get_byte_time_domain_data(&self) -> Vec<u8> {
        let mut result = vec![0u8; self.fft_size];
        let buf_len = self.time_domain_buffer.len();
        for i in 0..self.fft_size {
            let pos = (self.write_pos + i) % buf_len;
            // Map [-1, 1] to [0, 255]
            let val = ((self.time_domain_buffer[pos] + 1.0) * 128.0).clamp(0.0, 255.0);
            result[i] = val as u8;
        }
        result
    }

    /// Get the current time domain data as float values
    pub fn get_float_time_domain_data(&self) -> Vec<f32> {
        let mut result = vec![0.0f32; self.fft_size];
        let buf_len = self.time_domain_buffer.len();
        for i in 0..self.fft_size {
            let pos = (self.write_pos + i) % buf_len;
            result[i] = self.time_domain_buffer[pos];
        }
        result
    }
}

impl AudioNodeProcessor for AnalyserNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::Analyser
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

        // Pass through: copy input to output
        output[..len].copy_from_slice(&inputs[..len]);

        // Capture time-domain data (downmix to mono for analysis)
        let channels = self.channels.max(1) as usize;
        let frames = len / channels;
        let buf_len = self.time_domain_buffer.len();

        for frame in 0..frames {
            // Downmix to mono: average all channels
            let mut mono = 0.0f32;
            for ch in 0..channels {
                mono += inputs[frame * channels + ch];
            }
            mono /= channels as f32;

            self.time_domain_buffer[self.write_pos] = mono;
            self.write_pos = (self.write_pos + 1) % buf_len;
        }

        frames
    }
}
