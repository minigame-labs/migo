use std::sync::Arc;

use shared::protocol::audio_cmd::AudioNodeId;

use crate::decoder::DecodedAudio;

use super::AudioNode;

/// State of an AudioBufferSourceNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSourceState {
    /// Not yet started
    Pending,
    /// Currently playing
    Playing,
    /// Finished playback
    Finished,
}

pub struct BufferSourceNode {
    id: AudioNodeId,
    buffer: Option<Arc<DecodedAudio>>,
    state: BufferSourceState,

    // Playback position (in sample frames)
    position: f64,

    // Start parameters
    start_when: f64,
    start_offset: f64,
    duration: Option<f64>,

    // Loop parameters
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,

    // Playback rate
    playback_rate: f32,

    // Context time when started (set when start() is called)
    start_time: Option<f64>,
}

impl BufferSourceNode {
    pub fn new(id: AudioNodeId) -> Self {
        Self {
            id,
            buffer: None,
            state: BufferSourceState::Pending,
            position: 0.0,
            start_when: 0.0,
            start_offset: 0.0,
            duration: None,
            loop_enabled: false,
            loop_start: 0.0,
            loop_end: 0.0,
            playback_rate: 1.0,
            start_time: None,
        }
    }

    pub fn set_buffer(&mut self, buffer: Arc<DecodedAudio>) {
        self.buffer = Some(buffer);
    }

    pub fn start(&mut self, when: f64, offset: f64, duration: Option<f64>, current_time: f64) {
        if self.state != BufferSourceState::Pending {
            return; // Can only start once
        }

        self.start_when = when;
        self.start_offset = offset;
        self.duration = duration;
        self.position = offset;
        self.start_time = Some(current_time.max(when));
        self.state = BufferSourceState::Playing;
    }

    pub fn stop(&mut self, _when: f64) {
        self.state = BufferSourceState::Finished;
    }

    pub fn set_loop(&mut self, enabled: bool, start: f64, end: f64) {
        self.loop_enabled = enabled;
        self.loop_start = start;
        self.loop_end = end;
    }

    pub fn set_playback_rate(&mut self, rate: f32) {
        self.playback_rate = rate.max(0.0);
    }

    pub fn state(&self) -> BufferSourceState {
        self.state
    }
}

impl AudioNode for BufferSourceNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    /// Process audio with channel conversion support
    /// output_channels: number of channels in the output buffer (typically 2 for stereo)
    fn process(&mut self, output: &mut [f32], _sample_rate: u32) -> usize {
        self.process_with_channels(output, 2)
    }

    fn is_finished(&self) -> bool {
        self.state == BufferSourceState::Finished
    }

    fn output_channels(&self) -> u32 {
        self.buffer
            .as_ref()
            .map(|b| b.channels)
            .unwrap_or(2)
    }
}

impl BufferSourceNode {
    /// Process with explicit output channel count
    pub fn process_with_channels(&mut self, output: &mut [f32], output_channels: u32) -> usize {
        if self.state != BufferSourceState::Playing {
            return 0;
        }

        let buffer = match &self.buffer {
            Some(b) => b.clone(), // Clone Arc to release borrow
            None => return 0,
        };

        let src_channels = buffer.channels as usize;
        let dst_channels = output_channels as usize;
        let buffer_frames = buffer.frame_count();
        let output_frames = output.len() / dst_channels.max(1);
        let samples = &buffer.samples;
        let sample_rate = buffer.sample_rate as f64;

        // Dispatch to optimized path based on channel configuration
        match (src_channels, dst_channels) {
            (1, 2) => self.process_mono_to_stereo(output, samples, buffer_frames, output_frames, sample_rate),
            (2, 2) => self.process_stereo_to_stereo(output, samples, buffer_frames, output_frames, sample_rate),
            _ => self.process_generic(output, samples, buffer_frames, output_frames, src_channels, dst_channels, sample_rate),
        }
    }

    /// Check if playback is finished
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.state == BufferSourceState::Finished
    }

    /// Optimized path: mono source to stereo output
    #[inline]
    fn process_mono_to_stereo(
        &mut self,
        output: &mut [f32],
        samples: &[f32],
        buffer_frames: usize,
        output_frames: usize,
        sample_rate: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.playback_rate as f64;

        for frame_idx in 0..output_frames {
            let src_frame = self.position as usize;

            if src_frame >= buffer_frames {
                if self.loop_enabled {
                    self.position = self.loop_start * sample_rate;
                    continue;
                } else {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            if let Some(dur) = self.duration {
                if (self.position / sample_rate - self.start_offset) >= dur {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            let dst_idx = frame_idx * 2;

            // Bounds check for safety
            if src_frame < samples.len() && dst_idx + 1 < output.len() {
                let sample = samples[src_frame];
                output[dst_idx] = sample;
                output[dst_idx + 1] = sample;
            }

            self.position += playback_rate;
            frames_written += 1;
        }

        frames_written
    }

    /// Optimized path: stereo source to stereo output (most common case)
    #[inline]
    fn process_stereo_to_stereo(
        &mut self,
        output: &mut [f32],
        samples: &[f32],
        buffer_frames: usize,
        output_frames: usize,
        sample_rate: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.playback_rate as f64;

        for frame_idx in 0..output_frames {
            let src_frame = self.position as usize;

            if src_frame >= buffer_frames {
                if self.loop_enabled {
                    self.position = self.loop_start * sample_rate;
                    continue;
                } else {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            if let Some(dur) = self.duration {
                if (self.position / sample_rate - self.start_offset) >= dur {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            let src_idx = src_frame * 2;
            let dst_idx = frame_idx * 2;

            // Bounds check for safety
            if src_idx + 1 < samples.len() && dst_idx + 1 < output.len() {
                output[dst_idx] = samples[src_idx];
                output[dst_idx + 1] = samples[src_idx + 1];
            }

            self.position += playback_rate;
            frames_written += 1;
        }

        frames_written
    }

    /// Generic path for other channel configurations
    fn process_generic(
        &mut self,
        output: &mut [f32],
        samples: &[f32],
        buffer_frames: usize,
        output_frames: usize,
        src_channels: usize,
        dst_channels: usize,
        sample_rate: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.playback_rate as f64;

        for frame_idx in 0..output_frames {
            let src_frame = self.position as usize;

            if src_frame >= buffer_frames {
                if self.loop_enabled {
                    self.position = self.loop_start * sample_rate;
                    continue;
                } else {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            if let Some(dur) = self.duration {
                if (self.position / sample_rate - self.start_offset) >= dur {
                    self.state = BufferSourceState::Finished;
                    break;
                }
            }

            // Copy available channels with bounds checking
            let copy_channels = dst_channels.min(src_channels);
            let src_base = src_frame * src_channels;
            let dst_base = frame_idx * dst_channels;

            if src_base + copy_channels <= samples.len() && dst_base + dst_channels <= output.len() {
                for ch in 0..copy_channels {
                    output[dst_base + ch] = samples[src_base + ch];
                }

                // Fill extra output channels by duplicating last source channel
                if dst_channels > src_channels {
                    let last_sample = samples[src_base + src_channels - 1];
                    for ch in src_channels..dst_channels {
                        output[dst_base + ch] = last_sample;
                    }
                }
            }

            self.position += playback_rate;
            frames_written += 1;
        }

        frames_written
    }
}
