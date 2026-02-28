use deno_core::{Extension, extension, op2, v8};
use tracing::{debug, error, info, warn};

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
}
// TODO: remove
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
