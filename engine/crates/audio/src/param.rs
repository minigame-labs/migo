use std::cmp::Ordering;

/// Represents a scheduled automation event on an AudioParam timeline.
#[derive(Debug, Clone)]
enum AutomationEvent {
    /// Set the value at a specific time
    SetValue { value: f32, time: f64 },
    /// Linear ramp to a value ending at a specific time
    LinearRamp { value: f32, end_time: f64 },
    /// Exponential ramp to a value ending at a specific time
    ExponentialRamp { value: f32, end_time: f64 },
    /// Asymptotically approach a target value starting at a specific time
    SetTarget {
        target: f32,
        start_time: f64,
        time_constant: f64,
    },
    /// Cancel all scheduled events after a specific time
    #[allow(dead_code)]
    CancelScheduled { cancel_time: f64 },
}

impl AutomationEvent {
    fn time(&self) -> f64 {
        match self {
            Self::SetValue { time, .. } => *time,
            Self::LinearRamp { end_time, .. } => *end_time,
            Self::ExponentialRamp { end_time, .. } => *end_time,
            Self::SetTarget { start_time, .. } => *start_time,
            Self::CancelScheduled { cancel_time } => *cancel_time,
        }
    }
}

/// An audio parameter with support for scheduled value changes (automation).
///
/// Implements the W3C AudioParam specification for sample-accurate automation.
/// Events are stored in a sorted timeline and evaluated during audio processing.
#[derive(Debug, Clone)]
pub struct AudioParamTimeline {
    current_value: f32,
    default_value: f32,
    min_value: f32,
    max_value: f32,
    events: Vec<AutomationEvent>,
}

impl AudioParamTimeline {
    pub fn new(default_value: f32, min_value: f32, max_value: f32) -> Self {
        Self {
            current_value: default_value,
            default_value,
            min_value,
            max_value,
            events: Vec::new(),
        }
    }

    /// Get the current static value (without automation)
    #[inline]
    pub fn value(&self) -> f32 {
        self.current_value
    }

    /// Set the current value directly (equivalent to setting .value property)
    #[inline]
    pub fn set_value(&mut self, value: f32) {
        self.current_value = value.clamp(self.min_value, self.max_value);
    }

    pub fn default_value(&self) -> f32 {
        self.default_value
    }

    pub fn min_value(&self) -> f32 {
        self.min_value
    }

    pub fn max_value(&self) -> f32 {
        self.max_value
    }

    /// Schedule a value change at a specific time
    pub fn set_value_at_time(&mut self, value: f32, time: f64) {
        let value = value.clamp(self.min_value, self.max_value);
        self.insert_event(AutomationEvent::SetValue { value, time });
    }

    /// Schedule a linear ramp to a value ending at end_time
    pub fn linear_ramp_to_value_at_time(&mut self, value: f32, end_time: f64) {
        let value = value.clamp(self.min_value, self.max_value);
        self.insert_event(AutomationEvent::LinearRamp { value, end_time });
    }

    /// Schedule an exponential ramp to a value ending at end_time
    pub fn exponential_ramp_to_value_at_time(&mut self, value: f32, end_time: f64) {
        // Exponential ramp requires non-zero positive values
        let value = if value <= 0.0 {
            f32::EPSILON
        } else {
            value.clamp(f32::EPSILON, self.max_value)
        };
        self.insert_event(AutomationEvent::ExponentialRamp { value, end_time });
    }

    /// Asymptotically approach target starting at start_time with time_constant
    pub fn set_target_at_time(&mut self, target: f32, start_time: f64, time_constant: f64) {
        let target = target.clamp(self.min_value, self.max_value);
        self.insert_event(AutomationEvent::SetTarget {
            target,
            start_time,
            time_constant: time_constant.max(0.0001), // Prevent division by zero
        });
    }

    /// Cancel all scheduled events at or after cancel_time
    pub fn cancel_scheduled_values(&mut self, cancel_time: f64) {
        self.events.retain(|e| e.time() < cancel_time);
    }

    /// Check if there are any scheduled automation events
    #[inline]
    pub fn has_automation(&self) -> bool {
        !self.events.is_empty()
    }

    /// Compute the parameter value at a given context time.
    /// This evaluates the automation timeline and returns the correct value.
    pub fn compute_value(&self, current_time: f64) -> f32 {
        if self.events.is_empty() {
            return self.current_value;
        }

        let mut value = self.current_value;
        let mut prev_time = 0.0f64;
        let mut prev_value = self.current_value;

        for event in &self.events {
            match event {
                AutomationEvent::SetValue {
                    value: set_val,
                    time,
                } => {
                    if current_time >= *time {
                        value = *set_val;
                        prev_value = *set_val;
                        prev_time = *time;
                    }
                }
                AutomationEvent::LinearRamp {
                    value: end_val,
                    end_time,
                } => {
                    if current_time >= *end_time {
                        value = *end_val;
                        prev_value = *end_val;
                        prev_time = *end_time;
                    } else if current_time > prev_time {
                        let duration = *end_time - prev_time;
                        if duration > 0.0 {
                            let t = (current_time - prev_time) / duration;
                            value = prev_value + (*end_val - prev_value) * t as f32;
                        }
                    }
                }
                AutomationEvent::ExponentialRamp {
                    value: end_val,
                    end_time,
                } => {
                    if current_time >= *end_time {
                        value = *end_val;
                        prev_value = *end_val;
                        prev_time = *end_time;
                    } else if current_time > prev_time {
                        let duration = *end_time - prev_time;
                        if duration > 0.0 && prev_value.abs() > f32::EPSILON {
                            let t = (current_time - prev_time) / duration;
                            let ratio = *end_val / prev_value;
                            value = prev_value * ratio.powf(t as f32);
                        }
                    }
                }
                AutomationEvent::SetTarget {
                    target,
                    start_time,
                    time_constant,
                } => {
                    if current_time >= *start_time {
                        let elapsed = current_time - *start_time;
                        let exp = (-elapsed / *time_constant).exp() as f32;
                        value = *target + (prev_value - *target) * exp;
                    }
                }
                AutomationEvent::CancelScheduled { .. } => {
                    // Already handled by cancel_scheduled_values
                }
            }
        }

        value.clamp(self.min_value, self.max_value)
    }

    /// Fill a buffer with per-sample automation values.
    /// This is the high-performance path used during audio processing.
    ///
    /// `start_time`: context time at the start of the buffer
    /// `buffer`: output buffer to fill with parameter values
    /// `sample_rate`: audio sample rate for time calculation
    pub fn compute_values(&self, start_time: f64, buffer: &mut [f32], sample_rate: u32) {
        if self.events.is_empty() {
            // No automation — fill with constant value
            buffer.fill(self.current_value);
            return;
        }

        let inv_sample_rate = 1.0 / sample_rate as f64;
        let end_time = start_time + (buffer.len() as f64) * inv_sample_rate;

        // Fast path: all events are in the future — fill with current value
        if self.events.first().map_or(false, |e| e.time() > end_time) {
            buffer.fill(self.current_value);
            return;
        }

        for (i, sample) in buffer.iter_mut().enumerate() {
            let t = start_time + i as f64 * inv_sample_rate;
            *sample = self.compute_value(t);
        }
    }

    /// Clean up past events that are no longer needed (optimization).
    /// Call periodically to prevent unbounded event list growth.
    pub fn gc_events(&mut self, current_time: f64) {
        // Keep the most recent event before current_time and all future events.
        // We need the most recent past event to compute interpolation start values.
        if self.events.len() <= 1 {
            return;
        }

        let mut last_past_idx = None;
        for (i, event) in self.events.iter().enumerate() {
            if event.time() <= current_time {
                last_past_idx = Some(i);
            } else {
                break;
            }
        }

        if let Some(idx) = last_past_idx {
            if idx > 0 {
                // Update current_value to the evaluated value at this point
                self.current_value = self.compute_value(current_time);
                // Remove all past events except the most recent one
                self.events.drain(..idx);
            }
        }
    }

    /// Insert event in sorted order by time
    fn insert_event(&mut self, event: AutomationEvent) {
        let time = event.time();
        let pos = self
            .events
            .binary_search_by(|e| e.time().partial_cmp(&time).unwrap_or(Ordering::Equal))
            .unwrap_or_else(|pos| pos);
        self.events.insert(pos, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_value_at_time() {
        let mut param = AudioParamTimeline::new(0.5, 0.0, 1.0);
        param.set_value_at_time(0.8, 1.0);

        assert!((param.compute_value(0.5) - 0.5).abs() < f32::EPSILON);
        assert!((param.compute_value(1.0) - 0.8).abs() < f32::EPSILON);
        assert!((param.compute_value(2.0) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_linear_ramp() {
        let mut param = AudioParamTimeline::new(0.0, 0.0, 1.0);
        param.set_value_at_time(0.0, 0.0);
        param.linear_ramp_to_value_at_time(1.0, 1.0);

        assert!((param.compute_value(0.0) - 0.0).abs() < 0.01);
        assert!((param.compute_value(0.5) - 0.5).abs() < 0.01);
        assert!((param.compute_value(1.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_cancel_scheduled() {
        let mut param = AudioParamTimeline::new(0.0, 0.0, 1.0);
        param.set_value_at_time(0.5, 1.0);
        param.set_value_at_time(1.0, 2.0);
        param.cancel_scheduled_values(1.5);

        assert!((param.compute_value(1.0) - 0.5).abs() < f32::EPSILON);
        // Event at 2.0 should be cancelled
        assert!((param.compute_value(2.0) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_values_buffer() {
        let mut param = AudioParamTimeline::new(1.0, 0.0, 10.0);
        let mut buffer = vec![0.0f32; 4];
        param.compute_values(0.0, &mut buffer, 44100);
        assert!(buffer.iter().all(|&v| (v - 1.0).abs() < f32::EPSILON));
    }
}
