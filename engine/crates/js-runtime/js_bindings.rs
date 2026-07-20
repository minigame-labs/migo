use deno_core::v8;
use tracing::warn;

use shared::protocol::host_cmd::{TouchPoint, TouchType};

/// Cache of V8 `Global<Function>` handles for JS callbacks that the host
/// thread dispatches into frequently (touch, sensors, audio, etc.).
///
/// All callback fields (27 total) are `Option<v8::Global<v8::Function>>`.
/// They are populated during `reload()` by looking up `_internal*` functions
/// from the V8 global scope. A `None` value means the corresponding JS
/// function was not found (e.g., the extension is not loaded or the game
/// has not registered that API). Dispatch methods silently skip `None` fields.
///
/// ## Field groups
///
/// - **Touch / Input** (1): `enqueue_touch_event_fn`
/// - **Audio** (1): `enqueue_inner_audio_event_fn`
/// - **Recorder** (2): `recorder_event_fn`, `recorder_frame_fn`
/// - **Camera** (2): `camera_event_fn`, `camera_frame_fn`
/// - **Sensors** (6): device motion, gyroscope, accelerometer, compass, orientation, network
/// - **Bluetooth / Beacon** (4): adapter state, device found, beacon update, beacon service
/// - **BLE GATT** (3): connection state, characteristic value, MTU
/// - **System** (1): `memory_warning_fn`
/// - **Keyboard** (6): input, height, confirm, complete, key down, key up
/// - **Video** (1): `video_event_fn`
pub(crate) struct JsBindings {
    main_js_context: v8::Global<v8::Context>,

    /// Cached empty V8 string — avoids creating a new one on every fallback.
    empty_string: v8::Global<v8::String>,

    // ---- Touch / Input ----
    enqueue_touch_event_fn: Option<v8::Global<v8::Function>>,

    // ---- Audio ----
    enqueue_inner_audio_event_fn: Option<v8::Global<v8::Function>>,

    // ---- Recorder ----
    recorder_event_fn: Option<v8::Global<v8::Function>>,
    recorder_frame_fn: Option<v8::Global<v8::Function>>,

    // ---- Camera ----
    camera_event_fn: Option<v8::Global<v8::Function>>,
    camera_frame_fn: Option<v8::Global<v8::Function>>,

    // ---- Sensors (high-frequency; cached to avoid JS-side name lookup overhead) ----
    sensor_device_motion_fn: Option<v8::Global<v8::Function>>,
    sensor_gyroscope_fn: Option<v8::Global<v8::Function>>,
    sensor_accelerometer_fn: Option<v8::Global<v8::Function>>,
    sensor_compass_fn: Option<v8::Global<v8::Function>>,
    sensor_orientation_fn: Option<v8::Global<v8::Function>>,
    sensor_network_fn: Option<v8::Global<v8::Function>>,

    // ---- Bluetooth / Beacon ----
    bluetooth_adapter_state_change_fn: Option<v8::Global<v8::Function>>,
    bluetooth_device_found_fn: Option<v8::Global<v8::Function>>,
    beacon_update_fn: Option<v8::Global<v8::Function>>,
    beacon_service_change_fn: Option<v8::Global<v8::Function>>,

    // ---- BLE GATT ----
    ble_connection_state_change_fn: Option<v8::Global<v8::Function>>,
    ble_characteristic_value_change_fn: Option<v8::Global<v8::Function>>,
    ble_mtu_change_fn: Option<v8::Global<v8::Function>>,

    // ---- System ----
    memory_warning_fn: Option<v8::Global<v8::Function>>,

    // ---- WebGL context-loss lifecycle (webglcontextlost/restored) ----
    webgl_context_event_fn: Option<v8::Global<v8::Function>>,

    // ---- Keyboard (soft keyboard + physical key events) ----
    keyboard_input_fn: Option<v8::Global<v8::Function>>,
    keyboard_height_change_fn: Option<v8::Global<v8::Function>>,
    keyboard_confirm_fn: Option<v8::Global<v8::Function>>,
    keyboard_complete_fn: Option<v8::Global<v8::Function>>,
    key_down_fn: Option<v8::Global<v8::Function>>,
    key_up_fn: Option<v8::Global<v8::Function>>,
    gamepad_connected_fn: Option<v8::Global<v8::Function>>,
    gamepad_disconnected_fn: Option<v8::Global<v8::Function>>,
    gamepad_state_fn: Option<v8::Global<v8::Function>>,
    composition_start_fn: Option<v8::Global<v8::Function>>,
    composition_update_fn: Option<v8::Global<v8::Function>>,
    composition_end_fn: Option<v8::Global<v8::Function>>,

    // ---- Video ----
    video_event_fn: Option<v8::Global<v8::Function>>,
}

impl JsBindings {
    pub(crate) fn new(rt: &mut deno_core::JsRuntime, host_id: i32) -> Self {
        let main_js_context = rt.main_context();

        let empty_string = {
            let isolate = rt.v8_isolate();
            v8::scope_with_context!(scope, isolate, &main_js_context);
            let s = v8::String::empty(scope);
            v8::Global::new(scope, s)
        };

        let mut this = Self {
            main_js_context,
            empty_string,
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
            bluetooth_adapter_state_change_fn: None,
            bluetooth_device_found_fn: None,
            beacon_update_fn: None,
            beacon_service_change_fn: None,
            ble_connection_state_change_fn: None,
            ble_characteristic_value_change_fn: None,
            ble_mtu_change_fn: None,
            memory_warning_fn: None,
            webgl_context_event_fn: None,
            keyboard_input_fn: None,
            keyboard_height_change_fn: None,
            keyboard_confirm_fn: None,
            keyboard_complete_fn: None,
            key_down_fn: None,
            key_up_fn: None,
            gamepad_connected_fn: None,
            gamepad_disconnected_fn: None,
            gamepad_state_fn: None,
            composition_start_fn: None,
            composition_update_fn: None,
            composition_end_fn: None,
            video_event_fn: None,
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

        /// Resolve the Symbol-keyed host-bridge holder that `99_main.js`
        /// installs (`globalThis[Symbol.for('Migo.hostBridge')]`). The
        /// `_internal*` event-pump hooks live on this holder, not on the
        /// global, so an audit dump of globalThis does not surface them.
        /// Falls back to the global object if the holder is missing (e.g. a
        /// worker realm or an older bundle), preserving the prior behavior.
        fn resolve_host_bridge<'s>(
            scope: &v8::PinScope<'s, '_>,
            global: v8::Local<'s, v8::Object>,
        ) -> v8::Local<'s, v8::Object> {
            let Some(name) = v8::String::new(scope, "Migo.hostBridge") else {
                return global;
            };
            let sym = v8::Symbol::for_key(scope, name);
            match global.get(scope, sym.into()) {
                Some(v) => v8::Local::<v8::Object>::try_from(v).unwrap_or(global),
                None => global,
            }
        }

        let (
            enqueue_touch,
            enqueue_audio,
            rec_event,
            rec_frame,
            cam_event,
            cam_frame,
            dev_motion,
            gyro,
            accel,
            compass,
            orientation,
            network,
            bt_adapter_state,
            bt_device_found,
            beacon_update,
            beacon_svc_change,
            ble_conn_state,
            ble_char_value,
            ble_mtu,
            mem_warning,
            kb_input,
            kb_height,
            kb_confirm,
            kb_complete,
            key_down,
            key_up,
            video_event,
        ) = self.with_main_context(rt, |scope, _ctx, global| {
            // Hooks were relocated off the global onto the host-bridge holder
            // (see 99_main.js). Resolve it once, then look up every hook there.
            let bridge = resolve_host_bridge(scope, global);
            (
                get_global_fn(scope, bridge, "_internalEnqueueRawTouchEvent"),
                get_global_fn(scope, bridge, "_internalEnqueueInnerAudioEvent"),
                get_global_fn(scope, bridge, "_internalOnRecorderEvent"),
                get_global_fn(scope, bridge, "_internalOnRecorderFrameData"),
                get_global_fn(scope, bridge, "_internalOnCameraEvent"),
                get_global_fn(scope, bridge, "_internalOnCameraFrameData"),
                get_global_fn(scope, bridge, "_internalTriggerDeviceMotionChange"),
                get_global_fn(scope, bridge, "_internalTriggerGyroscopeChange"),
                get_global_fn(scope, bridge, "_internalTriggerAccelerometerChange"),
                get_global_fn(scope, bridge, "_internalTriggerCompassChange"),
                get_global_fn(scope, bridge, "_internalTriggerDeviceOrientationChange"),
                get_global_fn(scope, bridge, "_internalTriggerNetworkStatusChange"),
                get_global_fn(scope, bridge, "_internalTriggerBluetoothAdapterStateChange"),
                get_global_fn(scope, bridge, "_internalTriggerBluetoothDeviceFound"),
                get_global_fn(scope, bridge, "_internalTriggerBeaconUpdate"),
                get_global_fn(scope, bridge, "_internalTriggerBeaconServiceChange"),
                get_global_fn(scope, bridge, "_internalTriggerBLEConnectionStateChange"),
                get_global_fn(
                    scope,
                    bridge,
                    "_internalTriggerBLECharacteristicValueChange",
                ),
                get_global_fn(scope, bridge, "_internalTriggerBLEMTUChange"),
                get_global_fn(scope, bridge, "_internalTriggerMemoryWarning"),
                get_global_fn(scope, bridge, "_internalTriggerKeyboardInput"),
                get_global_fn(scope, bridge, "_internalTriggerKeyboardHeightChange"),
                get_global_fn(scope, bridge, "_internalTriggerKeyboardConfirm"),
                get_global_fn(scope, bridge, "_internalTriggerKeyboardComplete"),
                get_global_fn(scope, bridge, "_internalTriggerKeyDown"),
                get_global_fn(scope, bridge, "_internalTriggerKeyUp"),
                get_global_fn(scope, bridge, "_internalTriggerVideoEvent"),
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
        self.bluetooth_adapter_state_change_fn = bt_adapter_state;
        self.bluetooth_device_found_fn = bt_device_found;
        self.beacon_update_fn = beacon_update;
        self.beacon_service_change_fn = beacon_svc_change;
        self.ble_connection_state_change_fn = ble_conn_state;
        self.ble_characteristic_value_change_fn = ble_char_value;
        self.ble_mtu_change_fn = ble_mtu;
        self.memory_warning_fn = mem_warning;
        self.keyboard_input_fn = kb_input;
        self.keyboard_height_change_fn = kb_height;
        self.keyboard_confirm_fn = kb_confirm;
        self.keyboard_complete_fn = kb_complete;
        self.key_down_fn = key_down;
        self.key_up_fn = key_up;
        self.video_event_fn = video_event;

        // Resolved separately to avoid growing the tuple above; init-time only.
        let (
            webgl_context_event,
            gamepad_connected,
            gamepad_disconnected,
            gamepad_state,
            composition_start,
            composition_update,
            composition_end,
        ) = self
            .with_main_context(rt, |scope, _ctx, global| {
                let bridge = resolve_host_bridge(scope, global);
                (
                    get_global_fn(scope, bridge, "_internalTriggerWebglContextEvent"),
                    get_global_fn(scope, bridge, "_internalTriggerGamepadConnected"),
                    get_global_fn(scope, bridge, "_internalTriggerGamepadDisconnected"),
                    get_global_fn(scope, bridge, "_internalTriggerGamepadState"),
                    get_global_fn(scope, bridge, "_internalTriggerCompositionStart"),
                    get_global_fn(scope, bridge, "_internalTriggerCompositionUpdate"),
                    get_global_fn(scope, bridge, "_internalTriggerCompositionEnd"),
                )
            });
        self.webgl_context_event_fn = webgl_context_event;
        self.gamepad_connected_fn = gamepad_connected;
        self.gamepad_disconnected_fn = gamepad_disconnected;
        self.gamepad_state_fn = gamepad_state;
        self.composition_start_fn = composition_start;
        self.composition_update_fn = composition_update;
        self.composition_end_fn = composition_end;

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
                v8::String::new(scope, event_type)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into(),
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

    pub(crate) fn dispatch_accelerometer(
        &self,
        rt: &mut deno_core::JsRuntime,
        x: f64,
        y: f64,
        z: f64,
    ) {
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
                    v8::String::new(scope, accuracy)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_device_orientation(&self, rt: &mut deno_core::JsRuntime, value: &str) {
        if let Some(func_g) = self.sensor_orientation_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, value)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
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
                    v8::String::new(scope, network_type)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
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
                v8::String::new(scope, event_type)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into(),
                v8::String::new(scope, json_payload)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into(),
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

            let args = [ab.into(), v8::Boolean::new(scope, is_last_frame).into()];
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
                v8::String::new(scope, event_type)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into(),
                v8::String::new(scope, json_payload)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into(),
            ];
            let func = v8::Local::new(scope, func_g);
            let _ = func.call(scope, global.into(), &args);
        });
    }

    // ---- Bluetooth event dispatch ----

    /// Dispatch Bluetooth adapter state change event.
    pub(crate) fn dispatch_bluetooth_adapter_state_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        available: bool,
        discovering: bool,
    ) {
        if let Some(func_g) = self.bluetooth_adapter_state_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Boolean::new(scope, available).into(),
                    v8::Boolean::new(scope, discovering).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch Bluetooth device found event.
    pub(crate) fn dispatch_bluetooth_device_found(
        &self,
        rt: &mut deno_core::JsRuntime,
        devices_json: &str,
    ) {
        if let Some(func_g) = self.bluetooth_device_found_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, devices_json)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch Beacon update event.
    pub(crate) fn dispatch_beacon_update(&self, rt: &mut deno_core::JsRuntime, beacons_json: &str) {
        if let Some(func_g) = self.beacon_update_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, beacons_json)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch Beacon service change event.
    pub(crate) fn dispatch_beacon_service_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        available: bool,
        discovering: bool,
    ) {
        if let Some(func_g) = self.beacon_service_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Boolean::new(scope, available).into(),
                    v8::Boolean::new(scope, discovering).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    // ---- BLE GATT event dispatch ----

    /// Dispatch BLE connection state change event.
    pub(crate) fn dispatch_ble_connection_state_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        device_id: &str,
        connected: bool,
    ) {
        if let Some(func_g) = self.ble_connection_state_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::String::new(scope, device_id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::Boolean::new(scope, connected).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch BLE characteristic value change event.
    pub(crate) fn dispatch_ble_characteristic_value_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        device_id: &str,
        service_id: &str,
        characteristic_id: &str,
        value: &[u8],
    ) {
        if let Some(func_g) = self.ble_characteristic_value_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let ab = v8::ArrayBuffer::new(scope, value.len());
                if !value.is_empty() {
                    let backing = ab.get_backing_store();
                    if let Some(ptr) = backing.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                value.as_ptr(),
                                ptr.as_ptr() as *mut u8,
                                value.len(),
                            );
                        }
                    }
                }

                let args = [
                    v8::String::new(scope, device_id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, service_id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, characteristic_id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    ab.into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch BLE MTU change event.
    pub(crate) fn dispatch_ble_mtu_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        device_id: &str,
        mtu: u32,
    ) {
        if let Some(func_g) = self.ble_mtu_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::String::new(scope, device_id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::Integer::new(scope, mtu as i32).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    // ---- Memory warning dispatch ----

    /// Dispatch memory warning event.
    pub(crate) fn dispatch_memory_warning(&self, rt: &mut deno_core::JsRuntime, level: i32) {
        if let Some(func_g) = self.memory_warning_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::Integer::new(scope, level).into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    // ---- Keyboard event dispatch ----

    /// Dispatch keyboard input event (soft keyboard text changed).
    /// Fire a WebGL context-loss lifecycle event (`webglcontextlost` /
    /// `webglcontextrestored`) on the main canvas so the engine can drop and
    /// rebuild its GL resources. No-op if the JS hook was not resolved.
    ///
    /// The JS handler returns whether a listener called `preventDefault()`, but
    /// Migo's recovery is mandatory and automatic (driven entirely by the render
    /// thread), so the return value is intentionally NOT consumed to gate
    /// restoration — see `dispatchWebglContextEvent` in `web/03_canvas.js` for
    /// the non-spec recovery contract.
    pub(crate) fn dispatch_webgl_context_event(&self, rt: &mut deno_core::JsRuntime, kind: &str) {
        if let Some(func_g) = self.webgl_context_event_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, kind)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_keyboard_input(&self, rt: &mut deno_core::JsRuntime, value: &str) {
        if let Some(func_g) = self.keyboard_input_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, value)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch keyboard height change event.
    pub(crate) fn dispatch_keyboard_height_change(
        &self,
        rt: &mut deno_core::JsRuntime,
        height: f64,
    ) {
        if let Some(func_g) = self.keyboard_height_change_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::Number::new(scope, height).into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch keyboard confirm event.
    pub(crate) fn dispatch_keyboard_confirm(&self, rt: &mut deno_core::JsRuntime, value: &str) {
        if let Some(func_g) = self.keyboard_confirm_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, value)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch keyboard complete (dismiss) event.
    pub(crate) fn dispatch_keyboard_complete(&self, rt: &mut deno_core::JsRuntime, value: &str) {
        if let Some(func_g) = self.keyboard_complete_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, value)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch physical key down event (PC platform).
    pub(crate) fn dispatch_key_down(
        &self,
        rt: &mut deno_core::JsRuntime,
        key: &str,
        code: &str,
        timestamp_ms: f64,
    ) {
        if let Some(func_g) = self.key_down_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::String::new(scope, key)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, code)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::Number::new(scope, timestamp_ms).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch one IME composition event.
    ///
    /// The three share a body because they differ only in which listener group
    /// receives them: `data` means the same thing in each -- the whole current
    /// preedit, or the committed text at the end -- so a separate method per
    /// event would be three copies of one call.
    fn dispatch_composition(
        &self,
        rt: &mut deno_core::JsRuntime,
        func_g: Option<&v8::Global<v8::Function>>,
        data: &str,
    ) {
        if let Some(func_g) = func_g {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::String::new(scope, data)
                    .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                    .into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_composition_start(&self, rt: &mut deno_core::JsRuntime, data: &str) {
        self.dispatch_composition(rt, self.composition_start_fn.as_ref(), data);
    }

    pub(crate) fn dispatch_composition_update(&self, rt: &mut deno_core::JsRuntime, data: &str) {
        self.dispatch_composition(rt, self.composition_update_fn.as_ref(), data);
    }

    pub(crate) fn dispatch_composition_end(&self, rt: &mut deno_core::JsRuntime, data: &str) {
        self.dispatch_composition(rt, self.composition_end_fn.as_ref(), data);
    }

    /// Announce a gamepad. The id and mapping are the Web API's own fields.
    pub(crate) fn dispatch_gamepad_connected(
        &self,
        rt: &mut deno_core::JsRuntime,
        index: u32,
        id: &str,
        mapping: &str,
        axis_count: u8,
        button_count: u8,
    ) {
        if let Some(func_g) = self.gamepad_connected_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Integer::new_from_unsigned(scope, index).into(),
                    v8::String::new(scope, id)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, mapping)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::Integer::new_from_unsigned(scope, axis_count as u32).into(),
                    v8::Integer::new_from_unsigned(scope, button_count as u32).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    pub(crate) fn dispatch_gamepad_disconnected(
        &self,
        rt: &mut deno_core::JsRuntime,
        index: u32,
    ) {
        if let Some(func_g) = self.gamepad_disconnected_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [v8::Integer::new_from_unsigned(scope, index).into()];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Push one sample of a gamepad's axes and buttons.
    ///
    /// Packed into a single flat array rather than nested objects: this runs
    /// once per pad per frame while a pad is connected, and building a JS object
    /// per button would allocate a dozen short-lived objects every frame for a
    /// payload the JS side immediately copies into the state it already holds.
    ///
    /// Layout: `[axis_count, button_count, ...axes, ...(pressed, touched, value)]`.
    pub(crate) fn dispatch_gamepad_state(
        &self,
        rt: &mut deno_core::JsRuntime,
        state: &shared::protocol::host_cmd::GamepadState,
    ) {
        if let Some(func_g) = self.gamepad_state_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let axis_count = state.axis_count as usize;
                let button_count = state.button_count as usize;
                let packed = v8::Array::new(scope, (2 + axis_count + button_count * 3) as i32);

                let mut at = 0u32;
                let mut push = |scope: &v8::PinScope, value: f64| {
                    let number = v8::Number::new(scope, value);
                    packed.set_index(scope, at, number.into());
                    at += 1;
                };
                push(scope, axis_count as f64);
                push(scope, button_count as f64);
                for axis in &state.axes[..axis_count] {
                    push(scope, *axis as f64);
                }
                for button in &state.buttons[..button_count] {
                    push(scope, if button.pressed { 1.0 } else { 0.0 });
                    push(scope, if button.touched { 1.0 } else { 0.0 });
                    push(scope, button.value as f64);
                }

                let args = [
                    v8::Integer::new_from_unsigned(scope, state.index).into(),
                    v8::Number::new(scope, state.timestamp_ms).into(),
                    packed.into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch physical key up event (PC platform).
    pub(crate) fn dispatch_key_up(
        &self,
        rt: &mut deno_core::JsRuntime,
        key: &str,
        code: &str,
        timestamp_ms: f64,
    ) {
        if let Some(func_g) = self.key_up_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::String::new(scope, key)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, code)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::Number::new(scope, timestamp_ms).into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }

    /// Dispatch camera frame data (raw Y/U/V plane-window bytes concatenated in
    /// Y, U, V order, for onCameraFrame / listenFrameChange).
    pub(crate) fn dispatch_camera_frame_data(
        &self,
        rt: &mut deno_core::JsRuntime,
        host_id: i32,
        camera_id: u32,
        data: Vec<u8>,
        width: u32,
        height: u32,
    ) {
        let Some(func_g) = self.camera_frame_fn.as_ref() else {
            warn!("[Host {}] camera frame handler not installed", host_id);
            return;
        };

        self.with_main_context(rt, move |scope, _ctx, global| {
            // Hand the packed frame's allocation to V8 instead of copying it:
            // `new_backing_store_from_vec` adopts the `Vec`'s heap buffer as the
            // ArrayBuffer backing store, so there is no Rust->V8 copy. Valid
            // camera frames are non-empty; an empty frame degrades to a plain
            // empty ArrayBuffer.
            let ab = if data.is_empty() {
                v8::ArrayBuffer::new(scope, 0)
            } else {
                let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
                v8::ArrayBuffer::with_backing_store(scope, &store)
            };

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

    // ---- Video event dispatch ----

    /// Dispatch video player event to JS.
    /// Args: (video_id: u32, event_type: &str, data: &str)
    pub(crate) fn dispatch_video_event(
        &self,
        rt: &mut deno_core::JsRuntime,
        video_id: u32,
        event_type: &str,
        data: &str,
    ) {
        if let Some(func_g) = self.video_event_fn.as_ref() {
            self.with_main_context(rt, |scope, _ctx, global| {
                let args = [
                    v8::Integer::new(scope, video_id as i32).into(),
                    v8::String::new(scope, event_type)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                    v8::String::new(scope, data)
                        .unwrap_or_else(|| v8::Local::new(scope, &self.empty_string))
                        .into(),
                ];
                let func = v8::Local::new(scope, func_g);
                let _ = func.call(scope, global.into(), &args);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deno_core::{JsRuntime, RuntimeOptions};

    /// Real V8 regression for the camera-frame path: the frame `Vec<u8>` must be
    /// ADOPTED by V8 (its allocation becomes the ArrayBuffer backing) instead of
    /// copied. Records the `Vec`'s allocation pointer before the move, dispatches
    /// through the production `dispatch_camera_frame_data`, then reads the
    /// callback-visible `ArrayBuffer` back from Rust and asserts its backing
    /// pointer equals the original `Vec` pointer (RED under the old copy path),
    /// plus exact bytes, ArrayBuffer type/length, camera id, width, height.
    #[test]
    fn camera_frame_arraybuffer_adopts_vec_allocation_without_copy() {
        let mut rt = JsRuntime::new(RuntimeOptions::default());
        let mut bindings = JsBindings::new(&mut rt, 1);

        // Install a capturing camera callback and register it as the handler.
        let func = {
            let ctx = rt.main_context();
            let isolate = rt.v8_isolate();
            v8::scope_with_context!(scope, isolate, &ctx);
            let src = v8::String::new(
                scope,
                "(function(id, ab, w, h){ globalThis.__cam = { id: id, ab: ab, w: w, h: h }; })",
            )
            .unwrap();
            let script = v8::Script::compile(scope, src, None).unwrap();
            let val = script.run(scope).unwrap();
            let f = v8::Local::<v8::Function>::try_from(val).unwrap();
            v8::Global::new(scope, f)
        };
        bindings.camera_frame_fn = Some(func);

        // Record the Vec's allocation pointer BEFORE it is moved into dispatch.
        let bytes: Vec<u8> = vec![10, 20, 30, 0xFF, 0, 7, 200];
        let expected = bytes.clone();
        let orig_ptr = bytes.as_ptr();

        bindings.dispatch_camera_frame_data(&mut rt, 1, 42, bytes, 640, 480);

        // Read the callback-visible ArrayBuffer + scalars back from Rust.
        let ctx = rt.main_context();
        let isolate = rt.v8_isolate();
        v8::scope_with_context!(scope, isolate, &ctx);
        let context = v8::Local::new(scope, &ctx);
        let global = context.global(scope);

        let cam_key: v8::Local<v8::Value> = v8::String::new(scope, "__cam").unwrap().into();
        let cam = global.get(scope, cam_key).expect("__cam set by callback");
        let cam_obj = v8::Local::<v8::Object>::try_from(cam).expect("__cam is an object");

        let id_key: v8::Local<v8::Value> = v8::String::new(scope, "id").unwrap().into();
        let id = cam_obj
            .get(scope, id_key)
            .unwrap()
            .int32_value(scope)
            .unwrap();
        assert_eq!(id, 42, "camera id preserved");

        let w_key: v8::Local<v8::Value> = v8::String::new(scope, "w").unwrap().into();
        let w = cam_obj
            .get(scope, w_key)
            .unwrap()
            .int32_value(scope)
            .unwrap();
        assert_eq!(w, 640, "width preserved");

        let h_key: v8::Local<v8::Value> = v8::String::new(scope, "h").unwrap().into();
        let h = cam_obj
            .get(scope, h_key)
            .unwrap()
            .int32_value(scope)
            .unwrap();
        assert_eq!(h, 480, "height preserved");

        let ab_key: v8::Local<v8::Value> = v8::String::new(scope, "ab").unwrap().into();
        let ab_val = cam_obj.get(scope, ab_key).unwrap();
        assert!(
            ab_val.is_array_buffer(),
            "frame delivered as an ArrayBuffer"
        );
        let ab = v8::Local::<v8::ArrayBuffer>::try_from(ab_val).unwrap();
        assert_eq!(
            ab.byte_length(),
            expected.len(),
            "ArrayBuffer length == frame length"
        );

        let backing = ab.get_backing_store();
        let backing_ptr = backing.data().expect("non-empty backing store").as_ptr() as *const u8;

        // Exact bytes visible to JS.
        let mut got = vec![0u8; expected.len()];
        unsafe {
            std::ptr::copy_nonoverlapping(backing_ptr, got.as_mut_ptr(), expected.len());
        }
        assert_eq!(got, expected, "exact frame bytes visible to JS");

        // The regression: V8 adopts the moved Vec's allocation, so the backing
        // pointer equals the original Vec pointer. Under the old copy path these
        // differ (a fresh V8 allocation), which is the RED evidence.
        assert_eq!(
            backing_ptr, orig_ptr,
            "ArrayBuffer backing must be the moved Vec's allocation (zero-copy transfer)"
        );
    }
}
