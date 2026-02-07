use deno_core::{extension, Extension};

extension!(
    host_v8_event,
    esm = [
        dir "event",
        "01_event.js"
    ]
);

pub fn event_extensions() -> Vec<Extension> {
    vec![host_v8_event::init()]
}
