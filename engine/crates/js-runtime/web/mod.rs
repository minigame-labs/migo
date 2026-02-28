use deno_core::extension;
use shared::op_state::{CanvasOpState, HostOpState};
pub use timers::StartTime;

mod canvas;
mod timers;

extension!(
    host_v8_web,
    deps = [host_v8_console, host_v8_base, host_v8_webgl],
    ops = [
        timers::op_now,
        timers::op_now_us,
        canvas::op_create_canvas,
        canvas::op_get_canvas_info,
        canvas::op_resize_canvas,
        canvas::op_destroy_canvas,
    ],
    esm = [
        dir "web",
        "02_timers.js",
        "03_canvas.js",
        "06_stream.js",
        "12_location.js",
        "12_performance.js",
    ],
    state = |state| {
        let render_tx = state.borrow::<HostOpState>().render_tx.clone();
        state.put(StartTime::default());
        state.put(CanvasOpState {
            tx: render_tx,
            has_onscreen: false,
        });
    }
);

pub(crate) fn web_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_web::init()]
}

pub(crate) fn web_lazy_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_web::lazy_init()]
}
