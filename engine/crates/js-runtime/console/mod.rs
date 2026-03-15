use std::cell::Cell;

use deno_core::{Extension, extension, op2, v8};
use tracing::{debug, error, info, warn};

thread_local! {
    /// Host ID for the current JS runtime thread.
    /// Set once when the runtime is created; used by op_console to route
    /// log entries into the per-session console ring buffer (debug mode only).
    static HOST_ID: Cell<i32> = const { Cell::new(-1) };
}

/// Must be called from the host thread before the JS event loop starts.
pub fn set_thread_host_id(id: i32) {
    HOST_ID.set(id);
}

#[op2(fast)]
pub fn op_console<'s>(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>, level: u8) {
    let msg = value
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "<invalid value>".to_string());

    match level {
        1 => info!("{}", msg),
        2 => warn!("{}", msg),
        3 => error!("{}", msg),
        _ => debug!("{}", msg),
    }

    // Write to the per-session ring buffer (no-op if session has no buffer,
    // i.e. debug mode is disabled).
    HOST_ID.with(|cell| {
        let host_id = cell.get();
        if host_id >= 0 {
            shared::console_log::push_console_log(host_id, level, msg);
        }
    });
}
extension!(host_v8_console,
ops = [op_console],
esm = [
    dir "console",
    "01_console.js",
    "01_alert.js"
]
);

pub fn console_extensions() -> Vec<Extension> {
    vec![host_v8_console::init()]
}

pub fn console_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_console::lazy_init()]
}
