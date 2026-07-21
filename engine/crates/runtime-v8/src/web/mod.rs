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
        timers::op_timer_is_backgrounded,
        canvas::op_create_offscreen_canvas,
        canvas::op_get_canvas_info,
        canvas::op_resize_canvas,
        canvas::op_destroy_canvas,
    ],
    esm = [
        dir "src/web",
        "02_timers.js",
        "03_canvas.js",
        "06_stream.js",
        "12_performance.js",
    ],
    state = |state| {
        let host = state.borrow::<HostOpState>();
        let render_tx = host.render_tx.clone();
        // F-2: adopt the render-thread's shared TextMeasurer handle
        // so the JS-thread `op_measure_text_flat` fast path is
        // available without a cross-thread round-trip.
        let measurer = host.text_measurer.clone();
        state.put(StartTime::default());
        let mut canvas_state = CanvasOpState::new(render_tx);
        if let Some(m) = measurer {
            canvas_state = canvas_state.with_text_measurer(m);
        }
        state.put(canvas_state);
    }
);

pub(crate) fn web_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_web::init()]
}

pub(crate) fn web_lazy_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_web::lazy_init()]
}
