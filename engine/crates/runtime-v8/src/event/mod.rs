use deno_core::{Extension, extension};

extension!(
    host_v8_event,
    esm = [
        dir "src/event",
        "01_event.js"
    ]
);

pub fn event_extensions() -> Vec<Extension> {
    vec![host_v8_event::init()]
}

pub fn event_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_event::lazy_init()]
}
