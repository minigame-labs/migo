// web/timers.rs
use std::time::Instant;

use deno_core::{OpState, op2};

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
