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

    /// Radix-2 Cooley-Tukey in-place FFT.
    fn fft(re: &mut [f64], im: &mut [f64]) {
        let n = re.len();
        if n <= 1 {
            return;
        }
        debug_assert!(n.is_power_of_two());

        // Bit-reversal permutation
        let mut j = 0usize;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }

        // Butterfly passes
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let angle_step = -std::f64::consts::TAU / len as f64;
            let w_re = angle_step.cos();
            let w_im = angle_step.sin();

            let mut i = 0;
            while i < n {
                let mut cur_re = 1.0;
                let mut cur_im = 0.0;
                for k in 0..half {
                    let u_re = re[i + k];
                    let u_im = im[i + k];
                    let v_re = re[i + k + half] * cur_re - im[i + k + half] * cur_im;
                    let v_im = re[i + k + half] * cur_im + im[i + k + half] * cur_re;
                    re[i + k] = u_re + v_re;
                    im[i + k] = u_im + v_im;
                    re[i + k + half] = u_re - v_re;
                    im[i + k + half] = u_im - v_im;
                    let next_re = cur_re * w_re - cur_im * w_im;
                    cur_im = cur_re * w_im + cur_im * w_re;
                    cur_re = next_re;
                }
                i += len;
            }
            len <<= 1;
        }
    }

    /// Apply Blackman window to reduce spectral leakage.
    fn blackman_window(data: &mut [f64]) {
        let n = data.len();
        let inv_n = 1.0 / (n - 1) as f64;
        for i in 0..n {
            let w = 0.42 - 0.5 * (std::f64::consts::TAU * i as f64 * inv_n).cos()
                + 0.08 * (2.0 * std::f64::consts::TAU * i as f64 * inv_n).cos();
            data[i] *= w;
        }
    }

    /// Get frequency domain data in bytes (0-255), mapped from dB scale.
    pub fn get_byte_frequency_data(&self) -> Vec<u8> {
        let freq_bins = self.fft_size / 2;
        let buf_len = self.time_domain_buffer.len();

        // Prepare windowed time-domain data
        let mut re = vec![0.0f64; self.fft_size];
        let mut im = vec![0.0f64; self.fft_size];
        for i in 0..self.fft_size {
            let pos = (self.write_pos + i) % buf_len;
            re[i] = self.time_domain_buffer[pos] as f64;
        }
        Self::blackman_window(&mut re);

        Self::fft(&mut re, &mut im);

        let mut result = vec![0u8; freq_bins];
        let min_db = self.min_decibels as f64;
        let max_db = self.max_decibels as f64;
        let range = max_db - min_db;

        for i in 0..freq_bins {
            let magnitude = (re[i] * re[i] + im[i] * im[i]).sqrt() / self.fft_size as f64;
            let db = if magnitude > 1e-20 {
                20.0 * magnitude.log10()
            } else {
                min_db
            };
            // Map [minDecibels, maxDecibels] → [0, 255]
            let normalized = ((db - min_db) / range).clamp(0.0, 1.0);
            result[i] = (normalized * 255.0) as u8;
        }
        result
    }

    /// Get frequency domain data as float dB values.
    pub fn get_float_frequency_data(&self) -> Vec<f32> {
        let freq_bins = self.fft_size / 2;
        let buf_len = self.time_domain_buffer.len();

        let mut re = vec![0.0f64; self.fft_size];
        let mut im = vec![0.0f64; self.fft_size];
        for i in 0..self.fft_size {
            let pos = (self.write_pos + i) % buf_len;
            re[i] = self.time_domain_buffer[pos] as f64;
        }
        Self::blackman_window(&mut re);

        Self::fft(&mut re, &mut im);

        let mut result = vec![0.0f32; freq_bins];
        let min_db = self.min_decibels as f64;

        for i in 0..freq_bins {
            let magnitude = (re[i] * re[i] + im[i] * im[i]).sqrt() / self.fft_size as f64;
            let db = if magnitude > 1e-20 {
                20.0 * magnitude.log10()
            } else {
                min_db
            };
            result[i] = db.max(min_db) as f32;
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
