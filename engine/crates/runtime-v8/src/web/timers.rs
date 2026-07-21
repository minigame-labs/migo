// web/timers.rs
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use deno_core::{OpState, op2};
use shared::op_state::HostOpState;
use tokio::time::Instant;

pub struct StartTime(Instant);

impl Default for StartTime {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl std::ops::Deref for StartTime {
    type Target = Instant;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Write elapsed time since StartTime into `buf` as:
/// - u32 seconds (little-endian)
/// - u32 subsec_nanos (little-endian)
#[op2(fast)]
pub fn op_now(state: &mut OpState, #[buffer] buf: &mut [u8]) {
    let start_time = state.borrow::<StartTime>();
    let elapsed = start_time.elapsed();

    let seconds = elapsed.as_secs() as u32;
    let subsec_nanos = elapsed.subsec_nanos();

    if buf.len() >= 8 {
        buf[0..4].copy_from_slice(&seconds.to_le_bytes());
        buf[4..8].copy_from_slice(&subsec_nanos.to_le_bytes());
    }
}

/// Return the timer-specific lifecycle level. This is separate from network
/// throttling because foreground timer delivery may wait for a live Surface.
#[op2(fast)]
pub fn op_timer_is_backgrounded(state: &mut OpState) -> bool {
    state
        .borrow::<HostOpState>()
        .timer_backgrounded
        .load(Ordering::Acquire)
}

/// Return current Unix timestamp in microseconds as a f64.
#[op2(fast)]
#[number]
pub fn op_now_us() -> u64 {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_micros() as u64
}
