use std::sync::OnceLock;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{Registry, layer::SubscriberExt, util::SubscriberInitExt};

static LOG_INIT: OnceLock<()> = OnceLock::new();

/// Initialize tracing subscriber for Android (logcat).
/// Safe to call multiple times; it will only initialize once.
pub fn init_logging() {
    if LOG_INIT.set(()).is_err() {
        // Already initialized
        return;
    }

    // Android logcat layer.
    let android_layer =
        tracing_android::layer("[minigame-host]").expect("failed to create tracing_android layer");

    #[cfg(debug_assertions)]
    {
        // Optional timing layer for spans in debug builds.
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};
        use std::time::Instant;

        use tracing::{Id, Subscriber, field::Visit};
        use tracing_subscriber::layer::{Context, Layer};
        use tracing_subscriber::registry::LookupSpan;

        #[derive(Default)]
        struct FieldVisitor {
            fields: HashMap<String, String>,
        }

        impl Visit for FieldVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.fields
                    .insert(field.name().to_string(), format!("{:?}", value));
            }
        }

        struct TimingLayer {
            spans: Arc<Mutex<HashMap<Id, Instant>>>,
        }

        impl<S> Layer<S> for TimingLayer
        where
            S: Subscriber + for<'a> LookupSpan<'a>,
        {
            fn on_new_span(
                &self,
                attrs: &tracing::span::Attributes<'_>,
                id: &Id,
                ctx: Context<'_, S>,
            ) {
                if let Some(span) = ctx.span(id) {
                    let mut visitor = FieldVisitor::default();
                    attrs.record(&mut visitor);
                    span.extensions_mut().insert(visitor.fields);
                }
            }

            fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
                self.spans
                    .lock()
                    .unwrap()
                    .insert(id.clone(), Instant::now());
            }

            fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
                if let Some(start) = self.spans.lock().unwrap().remove(id) {
                    let dur = start.elapsed();
                    if let Some(span) = ctx.span(id) {
                        let name = span.name();
                        let ext = span.extensions();
                        let fields = ext.get::<HashMap<String, String>>();

                        if let Some(fields) = fields {
                            tracing::info!(target: "timing", "{} took {:?}, fields={:?}", name, dur, fields);
                        } else {
                            tracing::info!(target: "timing", "{} took {:?}", name, dur);
                        }
                    }
                }
            }
        }

        let timing_layer = TimingLayer {
            spans: Arc::new(Mutex::new(HashMap::new())),
        };

        Registry::default()
            .with(LevelFilter::TRACE)
            .with(android_layer)
            .with(timing_layer)
            .init();

        // Install panic hook for better crash logs in debug builds.
        // Note: We use Backtrace::force_capture() which captures backtrace regardless of
        // RUST_BACKTRACE env var, so no need to set it (which would be unsafe in multi-threaded context).
        std::panic::set_hook(Box::new(|info| {
            use std::backtrace::Backtrace;

            let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
                *s
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.as_str()
            } else {
                "<non-string panic payload>"
            };

            tracing::error!("panic: {}", payload);

            if let Some(location) = info.location() {
                tracing::error!(
                    "panic location: {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }

            let bt = Backtrace::force_capture();
            tracing::error!("backtrace:\n{:?}", bt);
        }));
    }

    #[cfg(not(debug_assertions))]
    {
        Registry::default()
            .with(LevelFilter::INFO)
            .with(android_layer)
            .init();
    }
}
