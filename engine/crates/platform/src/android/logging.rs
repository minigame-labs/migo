use std::sync::OnceLock;

use shared::config::LogLevel;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{Registry, layer::SubscriberExt, util::SubscriberInitExt};

static LOG_INIT: OnceLock<()> = OnceLock::new();

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

/// The filter for the record this thread is about to emit.
///
/// Resolved per thread rather than from one process-wide value, because a level
/// arrives per session and the sink does not: see `shared::log_level`, which owns
/// the three tiers and the reason a session must not be able to silence another.
fn load_active_filter() -> LevelFilter {
    log_level_to_filter(shared::log_level::effective_level())
}

/// Set the level for threads that belong to no session.
///
/// This is the process default, not "the current level": a session's own level is
/// published by its Host's registration and retired with it. A C host that
/// installs diagnostics without ever creating a session is the caller this exists
/// for.
pub fn update_log_level(level: LogLevel) {
    shared::log_level::set_default_level(level);
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
    let default_level = LogLevel::Debug;
    #[cfg(not(debug_assertions))]
    let default_level = LogLevel::Warn;

    shared::log_level::set_default_level(default_level);

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
    /// Keep every callsite undecided, so [`Self::enabled`] runs per event.
    ///
    /// `tracing` asks each callsite once what to think of it and then caches
    /// that answer for the life of the process. The default answer a `Layer`
    /// gives is derived from `enabled` at that moment -- which is exactly wrong
    /// for a filter whose level arrives later, from a session that does not
    /// exist yet. A callsite first reached while the process default was `Warn`
    /// was cached as `never` and stayed dead: a host that asked for `Info` got
    /// silence, and the startup timings it asked to see were the ones it could
    /// never get, because they are emitted while the host is being built. The
    /// mirror case is as bad -- a callsite first reached under `Trace` is cached
    /// as `always`, so a session asking for `Off` cannot silence it.
    ///
    /// `sometimes` is the answer for a filter that can change: it costs the
    /// per-event `enabled` call this module's documentation already assumes is
    /// happening.
    fn register_callsite(
        &self,
        _meta: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(
        &self,
        meta: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        meta.level() <= &load_active_filter()
    }
}
