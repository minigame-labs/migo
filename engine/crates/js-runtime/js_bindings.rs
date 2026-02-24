use deno_core::v8;
use tracing::warn;

use shared::protocol::host_cmd::{TouchPoint, TouchType};

/// Cache of JS globals / context handles that Host frequently calls.
/// Centralizes V8 scope handling to avoid borrow conflicts.
pub(crate) struct JsBindings {
    main_js_context: v8::Global<v8::Context>,
    enqueue_touch_event_fn: Option<v8::Global<v8::Function>>,
    enqueue_inner_audio_event_fn: Option<v8::Global<v8::Function>>,
    // Recorder event functions
    recorder_event_fn: Option<v8::Global<v8::Function>>,
    recorder_frame_fn: Option<v8::Global<v8::Function>>,
    // Camera event functions
    camera_event_fn: Option<v8::Global<v8::Function>>,
    camera_frame_fn: Option<v8::Global<v8::Function>>,
    // Sensor event functions (cached for high-frequency dispatch without JS parsing)
    sensor_device_motion_fn: Option<v8::Global<v8::Function>>,
    sensor_gyroscope_fn: Option<v8::Global<v8::Function>>,
    sensor_accelerometer_fn: Option<v8::Global<v8::Function>>,
    sensor_compass_fn: Option<v8::Global<v8::Function>>,
    sensor_orientation_fn: Option<v8::Global<v8::Function>>,
    sensor_network_fn: Option<v8::Global<v8::Function>>,
}

impl JsBindings {
    pub(crate) fn new(rt: &mut deno_core::JsRuntime, host_id: i32) -> Self {
        let main_js_context = rt.main_context();

        let mut this = Self {
            main_js_context,
            enqueue_touch_event_fn: None,
            enqueue_inner_audio_event_fn: None,
            recorder_event_fn: None,
            recorder_frame_fn: None,
            camera_event_fn: None,
            camera_frame_fn: None,
            sensor_device_motion_fn: None,
            sensor_gyroscope_fn: None,
            sensor_accelerometer_fn: None,
            sensor_compass_fn: None,
            sensor_orientation_fn: None,
            sensor_network_fn: None,
        };

        this.reload(rt, host_id);
        this
    }

    pub(crate) fn reload(&mut self, rt: &mut deno_core::JsRuntime, host_id: i32) {
        fn get_global_fn<'s>(
            scope: &v8::PinScope<'s, '_>,
            global: v8::Local<'s, v8::Object>,
            name: &'static str,
        ) -> Option<v8::Global<v8::Function>> {
            let key = v8::String::new(scope, name)?;
            let v = global.get(scope, key.into())?;
            let f = v8::Local::<v8::Function>::try_from(v).ok()?;
            Some(v8::Global::new(scope, f))
        }

        let (
            enqueue_touch, enqueue_audio,
            rec_event, rec_frame,
            cam_event, cam_frame,
            dev_motion, gyro, accel, compass, orientation, network,
        ) = self.with_main_context(rt, |scope, _ctx, global| {
            (
                get_global_fn(scope, global, "_internalEnqueueRawTouchEvent"),
                get_global_fn(scope, global, "_internalEnqueueInnerAudioEvent"),
                get_global_fn(scope, global, "_internalOnRecorderEvent"),
                get_global_fn(scope, global, "_internalOnRecorderFrameData"),
                get_global_fn(scope, global, "_internalOnCameraEvent"),
                get_global_fn(scope, global, "_internalOnCameraFrameData"),
                get_global_fn(scope, global, "_internalTriggerDeviceMotionChange"),
                get_global_fn(scope, global, "_internalTriggerGyroscopeChange"),
                get_global_fn(scope, global, "_internalTriggerAccelerometerChange"),
                get_global_fn(scope, global, "_internalTriggerCompassChange"),
                get_global_fn(scope, global, "_internalTriggerDeviceOrientationChange"),
                get_global_fn(scope, global, "_internalTriggerNetworkStatusChange"),
            )
        });

        self.enqueue_touch_event_fn = enqueue_touch;
        self.enqueue_inner_audio_event_fn = enqueue_audio;
        self.recorder_event_fn = rec_event;
        self.recorder_frame_fn = rec_frame;
        self.camera_event_fn = cam_event;
        self.camera_frame_fn = cam_frame;
        self.sensor_device_motion_fn = dev_motion;
        self.sensor_gyroscope_fn = gyro;
        self.sensor_accelerometer_fn = accel;
        self.sensor_compass_fn = compass;
        self.sensor_orientation_fn = orientation;
        self.sensor_network_fn = network;

        if self.enqueue_touch_event_fn.is_none() {
            warn!("[Host {}] _internalEnqueueRawTouchEvent not found", host_id);
        }
    }

    #[inline]
    fn with_main_context<R>(
        &self,
        rt: &mut deno_core::JsRuntime,
        f: impl for<'s, 'i> FnOnce(
            &v8::PinScope<'s, 'i>,
            v8::Local<'s, v8::Context>,
            v8::Local<'s, v8::Object>,
        ) -> R,
    ) -> R {
        let isolate = rt.v8_isolate();
        v8::scope_with_context!(scope, isolate, &self.main_js_context);
        let context = v8::Local::new(scope, &self.main_js_context);
        let global = context.global(scope);
        f(scope, context, global)
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

        let size = points.len() * std::mem::size_of::<TouchPoint>();

        self.with_main_context(rt, |scope, _ctx, global| {
            // Allocate ArrayBuffer directly in V8 heap — zero Rust heap allocation.
            // Single memcpy from points slice straight into V8-managed memory.
            let ab = v8::ArrayBuffer::new(scope, size);
            if size > 0 {
                let backing = ab.get_backing_store();
                if let Some(ptr) = backing.data() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            points.as_ptr() as *const u8,
                            ptr.as_ptr() as *mut u8,
                            size,
                        );
                    }
                }
            }

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

    /// Dispatch inner audio event to JS.
    /// Args: (id: u32, event_type: &str, current_time: f64)
    pub(crate) fn dispatch_inner_audio_event(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        id: u32,
        event_type: &str,
        current_time: f64,
    ) {
        let Some(func_g) = self.enqueue_inner_audio_event_fn.as_ref() else {
            warn!("[Host {}] inner audio event handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let args = [
                v8::Integer::new(scope, id as i32).into(),
                v8::String::new(scope, event_type).unwrap().into(),
                v8::Number::new(scope, current_time).into(),
            ];

            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    // ---- Sensor event dispatch (direct V8 calls, no JS parsing overhead) ----

    /// Dispatch a 3-float sensor event (DeviceMotion, Gyroscope, Accelerometer).
    #[inline]
    fn dispatch_f64x3(
        &self,
        rt: &mut deno_core::JsRuntime,
        func_g: &v8::Global<v8::Function>,
        a: f64,
        b: f64,
        c: f64,
    ) {
        self.with_main_context(rt, |scope, _ctx, global| {
            let args = [
                v8::Number::new(scope, a).into(),
                v8::Number::new(scope, b).into(),
                v8::Number::new(scope, c).into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    pub(crate) fn dispatch_device_motion(
        &self,
        rt: &mut deno_core::JsRuntime,
        alpha: f64,
        beta: f64,
        gamma: f64,
    ) {
        if let Some(f) = self.sensor_device_motion_fn.as_ref() {
            self.dispatch_f64x3(rt, f, alpha, beta, gamma);
        }
    }

    pub(crate) fn dispatch_gyroscope(&self, rt: &mut deno_core::JsRuntime, x: f64, y: f64, z: f64) {
        if let Some(f) = self.sensor_gyroscope_fn.as_ref() {
            self.dispatch_f64x3(rt, f, x, y, z);
        }
    }

    pub(crate) fn dispatch_accelerometer(&self, rt: &mut deno_core::JsRuntime, x: f64, y: f64, z: f64) {
        if let Some(f) = self.sensor_accelerometer_fn.as_ref() {
            self.dispatch_f64x3(rt, f, x, y, z);
        }
    }

    pub(crate) fn dispatch_compass(
        &self,
        rt: &mut deno_core::JsRuntime,
        direction: f64,
        accuracy: &str,
    ) {
        if let Some(func_g) = self.sensor_compass_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Number::new(scope, direction).into(),
                    v8::String::new(scope, accuracy).unwrap().into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_device_orientation(&self, rt: &mut deno_core::JsRuntime, value: &str) {
        if let Some(func_g) = self.sensor_orientation_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, value).unwrap().into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_network_status(
        &self,
        rt: &mut deno_core::JsRuntime,
        is_connected: bool,
        network_type: &str,
    ) {
        if let Some(func_g) = self.sensor_network_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Boolean::new(scope, is_connected).into(),
                    v8::String::new(scope, network_type).unwrap().into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    // ---- Recorder event dispatch ----

    /// Dispatch recorder event (start, pause, resume, stop, error, interruption).
    pub(crate) fn dispatch_recorder_event(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        event_type: &str,
        json_payload: &str,
    ) {
        let Some(func_g) = self.recorder_event_fn.as_ref() else {
            warn!("[Host {}] recorder event handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let args = [
                v8::String::new(scope, event_type).unwrap().into(),
                v8::String::new(scope, json_payload).unwrap().into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    /// Dispatch recorder frame data (binary audio frame for onFrameRecorded).
    pub(crate) fn dispatch_recorder_frame_data(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        data: &[u8],
        is_last_frame: bool,
    ) {
        let Some(func_g) = self.recorder_frame_fn.as_ref() else {
            warn!("[Host {}] recorder frame handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let ab = v8::ArrayBuffer::new(scope, data.len());
            if !data.is_empty() {
                let backing = ab.get_backing_store();
                if let Some(ptr) = backing.data() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            data.as_ptr(),
                            ptr.as_ptr() as *mut u8,
                            data.len(),
                        );
                    }
                }
            }

            let args = [
                ab.into(),
                v8::Boolean::new(scope, is_last_frame).into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    // ---- Camera event dispatch ----

    /// Dispatch camera event (stop, authCancel, error, timeoutCallback).
    pub(crate) fn dispatch_camera_event(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        camera_id: u32,
        event_type: &str,
        json_payload: &str,
    ) {
        let Some(func_g) = self.camera_event_fn.as_ref() else {
            warn!("[Host {}] camera event handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let args = [
                v8::Integer::new(scope, camera_id as i32).into(),
                v8::String::new(scope, event_type).unwrap().into(),
                v8::String::new(scope, json_payload).unwrap().into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    /// Dispatch camera frame data (RGBA pixel buffer for onCameraFrame / listenFrameChange).
    pub(crate) fn dispatch_camera_frame_data(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        camera_id: u32,
        data: &[u8],
        width: u32,
        height: u32,
    ) {
        let Some(func_g) = self.camera_frame_fn.as_ref() else {
            warn!("[Host {}] camera frame handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, |scope, _ctx, global| {
            let ab = v8::ArrayBuffer::new(scope, data.len());
            if !data.is_empty() {
                let backing = ab.get_backing_store();
                if let Some(ptr) = backing.data() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            data.as_ptr(),
                            ptr.as_ptr() as *mut u8,
                            data.len(),
                        );
                    }
                }
            }

            let args = [
                v8::Integer::new(scope, camera_id as i32).into(),
                ab.into(),
                v8::Integer::new(scope, width as i32).into(),
                v8::Integer::new(scope, height as i32).into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }
}
