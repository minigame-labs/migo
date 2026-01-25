/// Simple audio mixer for combining multiple sources
pub struct Mixer {
    channels: u32,
    mix_buffer: Vec<f32>,
}

impl Mixer {
    pub fn new(channels: u32) -> Self {
        Self {
            channels,
            mix_buffer: Vec::new(),
        }
    }

    /// Mix multiple audio sources into output buffer
    /// Each source is a slice of interleaved samples
    pub fn mix(&mut self, sources: &[&[f32]], output: &mut [f32]) {
        // Clear output
        output.fill(0.0);

        if sources.is_empty() {
            return;
        }

        // Sum all sources
        for source in sources {
            let len = source.len().min(output.len());
            for i in 0..len {
                output[i] += source[i];
            }
        }

        // Soft clipping to prevent harsh distortion
        for sample in output.iter_mut() {
            *sample = soft_clip(*sample);
        }
    }

    /// Get temporary buffer for processing
    pub fn get_buffer(&mut self, frames: usize) -> &mut [f32] {
        let size = frames * self.channels as usize;
        if self.mix_buffer.len() < size {
            self.mix_buffer.resize(size, 0.0);
        }
        &mut self.mix_buffer[..size]
    }
}

/// Soft clipping function to prevent harsh clipping
#[inline]
fn soft_clip(x: f32) -> f32 {
    if x > 1.0 {
        1.0 - 1.0 / (x + 1.0)
    } else if x < -1.0 {
        -1.0 + 1.0 / (-x + 1.0)
    } else {
        x
    }
}
