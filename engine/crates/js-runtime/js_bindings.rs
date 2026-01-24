use deno_core::v8;
use tracing::warn;

use shared::protocol::host_cmd::{TouchPoint, TouchType};

/// Cache of JS globals / context handles that Host frequently calls.
/// Centralizes V8 scope handling to avoid borrow conflicts.
pub(crate) struct JsBindings {
    main_js_context: v8::Global<v8::Context>,
    enqueue_touch_event_fn: Option<v8::Global<v8::Function>>,
    schedule_raf_fn: Option<v8::Global<v8::Function>>,
}

impl JsBindings {
    pub(crate) fn new(rt: &mut deno_core::JsRuntime, host_id: i32) -> Self {
        let main_js_context = rt.main_context();

        let mut this = Self {
            main_js_context,
            enqueue_touch_event_fn: None,
            schedule_raf_fn: None,
        };

        this.reload(rt, host_id);
        this
    }

    pub(crate) fn reload(&mut self, rt: &mut deno_core::JsRuntime, host_id: i32) {
        fn get_global_fn<'s>(
            scope: &mut v8::HandleScope<'s>,
            global: v8::Local<'s, v8::Object>,
            name: &'static str,
        ) -> Option<v8::Global<v8::Function>> {
            let key = v8::String::new(scope, name)?;
            let v = global.get(scope, key.into())?;
            let f = v8::Local::<v8::Function>::try_from(v).ok()?;
            Some(v8::Global::new(scope, f))
        }

        let (enqueue, raf) = self.with_main_context(rt, |scope, _ctx, global| {
            let enqueue = get_global_fn(scope, global, "_internalEnqueueRawTouchEvent");
            let raf = get_global_fn(scope, global, "_internalScheduleRaf");
            (enqueue, raf)
        });

        self.enqueue_touch_event_fn = enqueue;
        if self.enqueue_touch_event_fn.is_none() {
            warn!(
                "[Host {}] global function _internalEnqueueRawTouchEvent not found",
                host_id
            );
        }

        self.schedule_raf_fn = raf;
        if self.schedule_raf_fn.is_none() {
            warn!(
                "[Host {}] global function _internalScheduleRaf not found",
                host_id
            );
        }
    }

    #[inline]
    fn with_main_context<R>(
        &self,
        rt: &mut deno_core::JsRuntime,
        f: impl for<'s> FnOnce(
            &mut v8::HandleScope<'s>,
            v8::Local<'s, v8::Context>,
            v8::Local<'s, v8::Object>,
        ) -> R,
    ) -> R {
        let scope = &mut rt.handle_scope();
        let context = v8::Local::new(scope, &self.main_js_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        f(scope, context, global)
    }

    pub(crate) fn call_schedule_raf(&self, rt: &mut deno_core::JsRuntime) {
        let Some(func_g) = self.schedule_raf_fn.as_ref() else {
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &[]);
        });
    }

    pub(crate) fn dispatch_touch(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        touch_type: TouchType,
        points: &[TouchPoint],
        timestamp_ms: i64,
    ) {
        let Some(func_g) = self.enqueue_touch_event_fn.as_ref() else {
            warn!("[Host {}] touch handler not installed", host_id);
            return;
        };

        // TouchPoint slice -> bytes
        let size = points.len() * std::mem::size_of::<TouchPoint>();
        let mut bytes = vec![0u8; size];
        unsafe {
            std::ptr::copy_nonoverlapping(points.as_ptr() as *const u8, bytes.as_mut_ptr(), size);
        }

        self.with_main_context(rt, |scope, _ctx, global| {
            let backing =
                v8::ArrayBuffer::new_backing_store_from_boxed_slice(bytes.into_boxed_slice())
                    .make_shared();
            let ab = v8::ArrayBuffer::with_backing_store(scope, &backing);

            let args = [
                v8::Integer::new(scope, touch_type as i32).into(),
                ab.into(),
                v8::Integer::new(scope, points.len() as i32).into(),
                v8::Number::new(scope, timestamp_ms as f64).into(),
            ];

            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }
}
