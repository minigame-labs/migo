use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use shared::config::LogLevel;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{Registry, layer::SubscriberExt, util::SubscriberInitExt};

/// Stores the active log level as a `LevelFilter` discriminant.
/// Updated via `update_log_level()`; read on every tracing event.
static ACTIVE_LEVEL: AtomicU8 = AtomicU8::new(0); // 0 = OFF initially

static LOG_INIT: OnceLock<()> = OnceLock::new();

fn level_filter_to_u8(f: LevelFilter) -> u8 {
    match f {
        LevelFilter::OFF => 0,
        LevelFilter::ERROR => 1,
        LevelFilter::WARN => 2,
        LevelFilter::INFO => 3,
        LevelFilter::DEBUG => 4,
        LevelFilter::TRACE => 5,
    }
}

fn u8_to_level_filter(v: u8) -> LevelFilter {
    match v {
        0 => LevelFilter::OFF,
        1 => LevelFilter::ERROR,
        2 => LevelFilter::WARN,
        3 => LevelFilter::INFO,
        4 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

fn log_level_to_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Trace => LevelFilter::TRACE,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Off => LevelFilter::OFF,
    }
}

fn load_active_filter() -> LevelFilter {
    u8_to_level_filter(ACTIVE_LEVEL.load(Ordering::Relaxed))
}

/// Update the active log level at runtime.
///
/// Called when a new session is created with RuntimeConfig.
pub fn update_log_level(level: LogLevel) {
    let filter = log_level_to_filter(level);
    ACTIVE_LEVEL.store(level_filter_to_u8(filter), Ordering::Relaxed);
}

/// Initialize tracing subscriber for Android (logcat).
/// Safe to call multiple times; it will only initialize once.
///
/// Sets the default log level based on build type:
/// - debug builds: DEBUG
/// - release builds: WARN
pub fn init_logging() {
    if LOG_INIT.set(()).is_err() {
        return;
    }

    #[cfg(debug_assertions)]
    let default_level = LevelFilter::DEBUG;
    #[cfg(not(debug_assertions))]
    let default_level = LevelFilter::WARN;

    ACTIVE_LEVEL.store(level_filter_to_u8(default_level), Ordering::Relaxed);

    // Android logcat layer.
    let android_layer =
        tracing_android::layer("[migo]").expect("failed to create tracing_android layer");

    // Dynamic filter layer that reads the atomic level on each event.
    let dynamic_filter = DynamicLevelFilter;

    #[cfg(debug_assertions)]
    {
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

        // TRACE allows all events through; DynamicLevelFilter does the actual filtering.
        Registry::default()
            .with(LevelFilter::TRACE)
            .with(dynamic_filter)
            .with(android_layer)
            .with(timing_layer)
            .init();

        // Install panic hook for better crash logs in debug builds.
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
            .with(LevelFilter::TRACE)
            .with(dynamic_filter)
            .with(android_layer)
            .init();
    }
}

/// A layer that filters events based on the global atomic log level.
struct DynamicLevelFilter;

impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for DynamicLevelFilter {
    fn enabled(
        &self,
        meta: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        meta.level() <= &load_active_filter()
    }
}
