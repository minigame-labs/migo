use std::any::Any;
use std::sync::Arc;

use shared::audio_resources::{AudioBufferFormat, AudioSnapshot};
use shared::protocol::audio_cmd::AudioNodeId;

use crate::limits::RetainedAudio;
use crate::param::AudioParamTimeline;

use super::{AudioNodeProcessor, AudioNodeType};

/// State of an AudioBufferSourceNode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSourceState {
    /// Not yet started (start() not called)
    Pending,
    /// start() called but `when` is in the future
    Scheduled,
    /// Currently playing
    Playing,
    /// Finished playback
    Finished,
}

/// A source may still be backed by the legacy decoded-buffer registry, or by
/// the JS-owned snapshot path. Both variants retain the original Arc and
/// expose the same interleaved sample view; no PCM copy is made on binding.
enum SourceBuffer {
    Legacy(Arc<RetainedAudio>),
    Snapshot(Arc<AudioSnapshot>),
}

impl Clone for SourceBuffer {
    fn clone(&self) -> Self {
        match self {
            Self::Legacy(buffer) => Self::Legacy(Arc::clone(buffer)),
            Self::Snapshot(buffer) => Self::Snapshot(Arc::clone(buffer)),
        }
    }
}

impl SourceBuffer {
    #[inline]
    fn format(&self) -> AudioBufferFormat {
        match self {
            Self::Legacy(buffer) => AudioBufferFormat {
                channels: buffer.channels,
                frames: buffer.frame_count() as u32,
                sample_rate: buffer.sample_rate,
            },
            Self::Snapshot(buffer) => buffer.format(),
        }
    }

    #[inline]
    fn samples(&self) -> &[f32] {
        match self {
            Self::Legacy(buffer) => &buffer.samples,
            Self::Snapshot(buffer) => buffer.samples(),
        }
    }
}

pub struct BufferSourceNode {
    id: AudioNodeId,
    buffer: Option<SourceBuffer>,
    context_sample_rate: f64,
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

    // AudioParam for playback rate
    playback_rate: AudioParamTimeline,

    // Detune AudioParam (in cents)
    detune: AudioParamTimeline,

    // Context time when started (set when start() is called)
    start_time: Option<f64>,

    // Scheduled stop time (None = no stop scheduled)
    stop_when: Option<f64>,
}

impl BufferSourceNode {
    pub fn new(id: AudioNodeId, context_sample_rate: u32) -> Self {
        Self {
            id,
            buffer: None,
            context_sample_rate: context_sample_rate.max(1) as f64,
            state: BufferSourceState::Pending,
            position: 0.0,
            start_when: 0.0,
            start_offset: 0.0,
            duration: None,
            loop_enabled: false,
            loop_start: 0.0,
            loop_end: 0.0,
            playback_rate: AudioParamTimeline::new(1.0, -3.4028235e38, 3.4028235e38),
            detune: AudioParamTimeline::new(0.0, -3.4028235e38, 3.4028235e38),
            start_time: None,
            stop_when: None,
        }
    }

    pub fn set_buffer(&mut self, buffer: Option<Arc<RetainedAudio>>) {
        self.buffer = buffer.map(SourceBuffer::Legacy);
    }

    pub fn set_snapshot(&mut self, buffer: Option<Arc<AudioSnapshot>>) {
        self.buffer = buffer.map(SourceBuffer::Snapshot);
    }

    pub fn start(&mut self, when: f64, offset: f64, duration: Option<f64>, current_time: f64) {
        if self.state != BufferSourceState::Pending {
            return; // Can only start once per W3C spec
        }

        self.start_when = when;
        self.start_offset = offset;
        self.duration = duration;
        self.position = offset;

        // If scheduled time is now or in the past, start immediately
        if current_time >= when {
            self.start_time = Some(current_time);
            self.state = BufferSourceState::Playing;
        } else {
            // Schedule for future — will transition to Playing in process()
            self.start_time = Some(when);
            self.state = BufferSourceState::Scheduled;
        }
    }

    pub fn stop(&mut self, when: f64) {
        // Per W3C spec: stop at the given time, or immediately if when <= 0
        if when <= 0.0 {
            self.state = BufferSourceState::Finished;
        } else {
            self.stop_when = Some(when);
        }
    }

    pub fn set_loop(&mut self, enabled: bool, start: f64, end: f64) {
        self.loop_enabled = enabled;
        self.loop_start = start;
        self.loop_end = end;
    }

    /// Frame index at which playback wraps: the loop end (clamped to the buffer)
    /// when looping with a valid `loopEnd`, otherwise the end of the buffer. A
    /// `loopEnd` of 0 or <= `loopStart` means "use the whole buffer" (Web Audio).
    #[inline]
    fn loop_wrap_frame(&self, buffer_frames: usize, sample_rate: f64) -> usize {
        if self.loop_enabled && self.loop_end > self.loop_start && self.loop_end > 0.0 {
            ((self.loop_end * sample_rate) as usize).min(buffer_frames)
        } else {
            buffer_frames
        }
    }

    /// Frame index to jump back to when looping. Clamps `loopStart` to a valid
    /// frame; an out-of-range `loopStart` restarts at 0 (full-buffer loop) rather
    /// than jumping past the buffer and looping silence.
    #[inline]
    fn loop_restart_pos(&self, buffer_frames: usize, sample_rate: f64) -> f64 {
        let start_frame = (self.loop_start.max(0.0) * sample_rate).floor();
        if start_frame >= buffer_frames as f64 {
            0.0
        } else {
            start_frame
        }
    }

    pub fn set_playback_rate(&mut self, rate: f32) {
        self.playback_rate.set_value(rate.max(0.0));
    }

    /// Compute the source-frame increment once per render block. Web Audio's
    /// intrinsic rate is playbackRate × detune pitch ratio, then converted
    /// from buffer frames to context frames for buffers at another rate.
    #[inline]
    fn block_playback_rate(&self, current_time: f64, buffer_sample_rate: f64) -> f64 {
        let playback_rate = self.playback_rate.compute_value(current_time).max(0.0) as f64;
        let detune = self.detune.compute_value(current_time) as f64;
        playback_rate
            * (2.0f64).powf(detune / 1200.0)
            * (buffer_sample_rate / self.context_sample_rate)
    }

    #[allow(dead_code)]
    pub fn state(&self) -> BufferSourceState {
        self.state
    }

    /// Process with explicit output channel count
    pub fn process_with_channels(
        &mut self,
        output: &mut [f32],
        output_channels: u32,
        current_time: f64,
    ) -> usize {
        if self.state != BufferSourceState::Playing {
            return 0;
        }

        let buffer = match &self.buffer {
            Some(b) => b.clone(), // Clone Arc to release borrow
            None => return 0,
        };

        let format = buffer.format();
        let src_channels = format.channels as usize;
        let dst_channels = output_channels as usize;
        let buffer_frames = format.frames as usize;
        let output_frames = output.len() / dst_channels.max(1);
        let samples = buffer.samples();
        let sample_rate = format.sample_rate as f64;

        // Dispatch to optimized path based on channel configuration
        match (src_channels, dst_channels) {
            (1, 2) => self.process_mono_to_stereo(
                output,
                samples,
                buffer_frames,
                output_frames,
                sample_rate,
                current_time,
            ),
            (2, 2) => self.process_stereo_to_stereo(
                output,
                samples,
                buffer_frames,
                output_frames,
                sample_rate,
                current_time,
            ),
            _ => self.process_generic(
                output,
                samples,
                buffer_frames,
                output_frames,
                src_channels,
                dst_channels,
                sample_rate,
                current_time,
            ),
        }
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
        current_time: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.block_playback_rate(current_time, sample_rate);

        for frame_idx in 0..output_frames {
            let mut src_frame = self.position as usize;

            let wrap_frame = self.loop_wrap_frame(buffer_frames, sample_rate);
            if src_frame >= wrap_frame {
                if self.loop_enabled {
                    // Wrap and render the restart sample this iteration (a `continue`
                    // would drop a frame), carrying the fractional/rate overshoot so
                    // high or fractional playback rates keep correct loop timing.
                    let restart = self.loop_restart_pos(buffer_frames, sample_rate);
                    let loop_len = (wrap_frame as f64 - restart).max(1.0);
                    let overshoot = (self.position - wrap_frame as f64).max(0.0);
                    self.position = restart + overshoot.rem_euclid(loop_len);
                    src_frame = self.position as usize;
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
        current_time: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.block_playback_rate(current_time, sample_rate);

        for frame_idx in 0..output_frames {
            let mut src_frame = self.position as usize;

            let wrap_frame = self.loop_wrap_frame(buffer_frames, sample_rate);
            if src_frame >= wrap_frame {
                if self.loop_enabled {
                    // Wrap and render the restart sample this iteration (a `continue`
                    // would drop a frame), carrying the fractional/rate overshoot so
                    // high or fractional playback rates keep correct loop timing.
                    let restart = self.loop_restart_pos(buffer_frames, sample_rate);
                    let loop_len = (wrap_frame as f64 - restart).max(1.0);
                    let overshoot = (self.position - wrap_frame as f64).max(0.0);
                    self.position = restart + overshoot.rem_euclid(loop_len);
                    src_frame = self.position as usize;
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
        current_time: f64,
    ) -> usize {
        let mut frames_written = 0;
        let playback_rate = self.block_playback_rate(current_time, sample_rate);

        for frame_idx in 0..output_frames {
            let mut src_frame = self.position as usize;

            let wrap_frame = self.loop_wrap_frame(buffer_frames, sample_rate);
            if src_frame >= wrap_frame {
                if self.loop_enabled {
                    // Wrap and render the restart sample this iteration (a `continue`
                    // would drop a frame), carrying the fractional/rate overshoot so
                    // high or fractional playback rates keep correct loop timing.
                    let restart = self.loop_restart_pos(buffer_frames, sample_rate);
                    let loop_len = (wrap_frame as f64 - restart).max(1.0);
                    let overshoot = (self.position - wrap_frame as f64).max(0.0);
                    self.position = restart + overshoot.rem_euclid(loop_len);
                    src_frame = self.position as usize;
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

            if src_base + copy_channels <= samples.len() && dst_base + dst_channels <= output.len()
            {
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

impl AudioNodeProcessor for BufferSourceNode {
    fn id(&self) -> AudioNodeId {
        self.id
    }

    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::BufferSource
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn process(
        &mut self,
        _inputs: &[f32],
        output: &mut [f32],
        _sample_rate: u32,
        channels: u32,
        current_time: f64,
    ) -> usize {
        // Check scheduled start: transition Scheduled → Playing when time arrives
        if self.state == BufferSourceState::Scheduled {
            if current_time >= self.start_when {
                self.state = BufferSourceState::Playing;
            } else {
                return 0;
            }
        }

        // Check scheduled stop
        if let Some(stop_when) = self.stop_when {
            if current_time >= stop_when {
                self.state = BufferSourceState::Finished;
                return 0;
            }
        }

        // Source node: ignore inputs, generate from buffer
        self.process_with_channels(output, channels, current_time)
    }

    fn is_finished(&self) -> bool {
        self.state == BufferSourceState::Finished
    }

    fn is_source(&self) -> bool {
        true
    }

    fn output_channels(&self) -> u32 {
        self.buffer
            .as_ref()
            .map(|buffer| buffer.format().channels)
            .unwrap_or(2)
    }

    fn get_param_mut(&mut self, name: &str) -> Option<&mut AudioParamTimeline> {
        match name {
            "playbackRate" => Some(&mut self.playback_rate),
            "detune" => Some(&mut self.detune),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_rate_includes_detune_and_sample_rate_conversion() {
        let mut node = BufferSourceNode::new(1, 48_000);
        node.set_playback_rate(2.0);
        node.detune.set_value(1_200.0);

        let actual = node.block_playback_rate(0.0, 44_100.0);
        let expected = 2.0 * 2.0 * 44_100.0 / 48_000.0;
        assert!((actual - expected).abs() < f64::EPSILON * 32.0);
    }
}
