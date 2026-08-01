//! # Host Command Protocol
//!
//! Defines commands sent to the host runtime thread from other engine components.
//! The host thread runs the JavaScript runtime and coordinates the game lifecycle.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────┐     HostCommand      ┌──────────────┐
//! │ Render Thread├─────────────────────►│              │
//! └──────────────┘                      │              │
//! ┌──────────────┐     HostCommand      │  Host Thread │
//! │ Audio Thread ├─────────────────────►│  (JS Runtime)│
//! └──────────────┘                      │              │
//! ┌──────────────┐     HostCommand      │              │
//! │ Platform/JNI ├─────────────────────►│              │
//! └──────────────┘                      └──────────────┘
//! ```
//!
//! ## Command Categories
//!
//! The `HostCommand` enum is the authoritative list; the grouping and counts
//! below are indicative and may lag as variants (e.g. `SurfaceDestroyed`) are
//! added.
//!
//! - **Module Loading** (2): `EvaluateModule`, `EvalScript`
//! - **Lifecycle** (4): `Restart`, `Shutdown`, `OnShow`, `OnHide`
//! - **Audio** (3): `OnAudioInterruptionBegin`, `OnAudioInterruptionEnd`, `InnerAudioEvent`
//! - **Rendering / Surface** (1): `UpdateSurface`
//! - **Touch / Input** (1): `OnTouch`
//! - **Desktop pointer** (4): `OnMouseDown` .. `OnWheel`
//! - **Sensor Events** (5): `OnDeviceMotionChange` .. `OnAccelerometerChange`
//! - **Network** (1): `OnNetworkStatusChange`
//! - **Recorder** (2): `RecorderEvent`, `RecorderFrameData`
//! - **Camera** (2): `CameraEvent`, `CameraFrameData`
//! - **Keyboard** (6): `OnKeyboardInput` .. `OnKeyUp`
//! - **Gamepad** (3): `OnGamepadConnected` .. `OnGamepadState`
//! - **IME composition** (3): `OnCompositionStart` .. `OnCompositionEnd`
//! - **Bluetooth / BLE** (7): `OnBluetoothAdapterStateChange` .. `OnBeaconServiceChange`
//! - **Video** (1): `OnVideoStateChange`
//! - **System** (2): `OnMemoryWarning`, `OnUserCaptureScreen`

use std::borrow::Cow;

use crate::{
    payload_pool::Pooled,
    surface::{
        PixelRatio, PublicSurfaceGeneration, SurfaceGeneration, SurfaceLease, SurfaceLossReason,
    },
};

/// Touch event payload, stored in a preallocated `HostCommand::OnTouch` slot.
///
/// Contains a fixed `[TouchPoint; 10]` array with a count field.
/// Single memcpy from JNI DirectByteBuffer into a bounded, preallocated
/// payload slot keeps steady-state input allocation-free.
#[derive(Debug)]
pub struct TouchData {
    /// Type of touch event (start, move, end, cancel).
    pub touch_type: TouchType,
    /// Number of valid touch points in the `points` array.
    pub count: u8,
    /// Fixed inline array of touch points (max 10 simultaneous).
    /// Only `points[..count]` are valid.
    pub points: [TouchPoint; 10],
    /// Event timestamp in milliseconds (from system boot or epoch).
    pub timestamp_ms: i64,
}

/// The largest gamepad this runtime carries state for.
///
/// The W3C standard mapping has 4 axes and 17 buttons; the headroom covers
/// devices that report a few more without making the payload variable-length.
/// The public ABI rejects a topology past these limits instead of silently
/// truncating buttons or axes that content may rely on; a platform adapter may
/// deliberately remap a device before it enters this transport.
pub const GAMEPAD_MAX_AXES: usize = 8;
pub const GAMEPAD_MAX_BUTTONS: usize = 20;

/// One button's state, matching the Web `GamepadButton`.
///
/// `value` is the analogue position for a trigger and 0.0/1.0 for a digital
/// button; `pressed` is not derivable from it, because a device chooses its own
/// press threshold and content must not have to guess one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GamepadButtonState {
    pub pressed: bool,
    pub touched: bool,
    pub value: f32,
}

/// A gamepad's current state, boxed inside `HostCommand::OnGamepadState`.
///
/// Fixed inline arrays with counts, like `TouchData`: this arrives once per pad
/// per frame while a pad is connected, and a `Vec` here would allocate on the
/// input path for a payload whose maximum size is under 200 bytes.
#[derive(Debug)]
pub struct GamepadState {
    /// The slot this pad occupies, matching its index in `getGamepads()`.
    pub index: u32,
    /// Valid entries in `axes`.
    pub axis_count: u8,
    /// Valid entries in `buttons`.
    pub button_count: u8,
    /// Each in -1.0..=1.0, in the standard mapping's order.
    pub axes: [f32; GAMEPAD_MAX_AXES],
    pub buttons: [GamepadButtonState; GAMEPAD_MAX_BUTTONS],
    /// When the host sampled this state, in milliseconds.
    pub timestamp_ms: f64,
}

/// BLE characteristic value change payload, boxed inside
/// `HostCommand::OnBLECharacteristicValueChange`.
///
/// Contains three String UUIDs plus a variable-length byte buffer,
/// boxed to keep the `HostCommand` enum small on the channel.
#[derive(Debug)]
pub struct BleCharacteristicData {
    /// BLE device identifier.
    pub device_id: String,
    /// GATT service UUID.
    pub service_id: String,
    /// GATT characteristic UUID.
    pub characteristic_id: String,
    /// Characteristic value bytes.
    pub value: Vec<u8>,
}

/// Commands sent to the host runtime thread.
///
/// These commands drive the JavaScript runtime and coordinate between
/// native subsystems (rendering, audio, input) and the JS game code.
///
/// # Variant Groups
///
/// The enum below is the authoritative list; the grouping and counts here are
/// indicative and may lag as variants (e.g. `SurfaceDestroyed`) are added.
///
/// - **Module Loading** (2): `EvaluateModule`, `EvalScript`
/// - **Lifecycle** (4): `Restart`, `Shutdown`, `OnShow`, `OnHide`
/// - **Rendering / Surface** (1): `UpdateSurface`
/// - **Touch / Input** (1): `OnTouch`
/// - **Desktop pointer** (4): `OnMouseDown` .. `OnWheel`
/// - **Keyboard Events** (6): `OnKeyboardInput` .. `OnKeyUp`
/// - **Gamepad Events** (3): `OnGamepadConnected` .. `OnGamepadState`
/// - **IME Composition Events** (3): `OnCompositionStart` .. `OnCompositionEnd`
/// - **Sensor Events** (5): `OnDeviceMotionChange` .. `OnAccelerometerChange`
/// - **Network** (1): `OnNetworkStatusChange`
/// - **Audio Events** (3): `OnAudioInterruptionBegin`, `OnAudioInterruptionEnd`, `InnerAudioEvent`
/// - **Recorder Events** (2): `RecorderEvent`, `RecorderFrameData`
/// - **Camera Events** (2): `CameraEvent`, `CameraFrameData`
/// - **Bluetooth / BLE Events** (7): `OnBluetoothAdapterStateChange` .. `OnBeaconServiceChange`
/// - **Video Events** (1): `OnVideoStateChange`
/// - **System Events** (3): `OnMemoryWarning`, `OnUserCaptureScreen`, `OnThermalStatusChanged`
///
/// # Thread Safety
///
/// Commands are sent via `tokio::sync::mpsc::Sender` and processed
/// asynchronously by the host thread's event loop.
///
/// # Example
///
/// ```rust,ignore
/// use shared::protocol::host_cmd::HostCommand;
///
/// // Start a game
/// let cmd = HostCommand::EvaluateModule {
///     dir: “/data/game”.to_string(),
///     entry: “main.js”.to_string(),
/// };
///
/// // Send touch event
/// let cmd = HostCommand::OnTouch(Box::new(TouchData {
///     touch_type: TouchType::Start,
///     count: 1,
///     points: Default::default(), // filled via ptr::copy_nonoverlapping
///     timestamp_ms: 1234567890,
/// }));
/// ```
#[non_exhaustive]
#[derive(Debug)]
pub enum HostCommand {
    // ---- Module Loading ----
    /// Evaluate an ES module with isolated VFS paths.
    ///
    /// This is the primary way to start a mini-game. The module is loaded
    /// from the game's code directory, and sandboxed file access is enabled.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// HostCommand::EvaluateModule {
    ///     game_id: “my-puzzle-game”.to_string(),
    ///     entry: “game.js”.to_string(),
    /// }
    /// ```
    EvaluateModule {
        /// Unique game identifier (1-64 alphanumeric, underscore, hyphen).
        /// Used to derive all game paths from base directories.
        game_id: String,
        /// Entry point module filename (e.g., “main.js”, “game.js”).
        entry: String,
    },

    /// Evaluate a JS snippet (non-module).
    ///
    /// Useful for debugging or dynamic code execution. The code runs
    /// in the global scope, not as a module. Also used by Mode C async
    /// APIs (EvalScript pattern) to deliver platform callback results.
    EvalScript {
        /// JavaScript source code to execute.
        source: String,
    },

    /// Call one host-bridge hook by name, arguments encoded as a JSON array.
    ///
    /// The replacement for delivering host callbacks as [`Self::EvalScript`]
    /// source. That source has to name
    /// `globalThis[Symbol.for('Migo.hostBridge')]`, and `Symbol.for` reads the
    /// *global* symbol registry -- so the holder is reachable by content, which
    /// can then call any hook on it. Dispatching through a handle the runtime
    /// resolved at start-up needs no name at all.
    InvokeHostHook {
        /// Hook name on the host bridge, e.g. `_internalOnLoginResult`.
        hook: String,
        /// Arguments as a JSON array. Covers every shape in use: `[]` for no
        /// arguments, `["{...}"]` for a JSON string, `[{...}]` for an object,
        /// `[1, 0]` for numbers.
        args_json: String,
    },

    // ---- Lifecycle Events ----
    /// Restart the current game runtime.
    ///
    /// Re-initializes the JS runtime and re-loads the last evaluated module
    /// (or does nothing if none has been evaluated). This is analogous to
    /// mini-program “restart” behavior.
    Restart,

    /// Gracefully shut down the host thread.
    ///
    /// This triggers cleanup of all resources and terminates the event loop.
    Shutdown,

    /// Notify that the app has become visible/active.
    ///
    /// Triggers `migo.onShow` callbacks in the game.
    ///
    /// `options_json` should be a JSON object string with launch/enter params
    /// (e.g. scene/query/referrerInfo/shareTicket). If absent or invalid, JS
    /// will fall back to default launch options.
    OnShow { options_json: Option<String> },

    /// Notify that the app has become hidden/inactive.
    ///
    /// Triggers `migo.onHide` callbacks in the game.
    OnHide,

    /// Window/input focus changed independently of app visibility.
    ///
    /// The runtime forwards this into a profile-neutral adapter hook. HTML5
    /// adapters translate it to window focus/blur; wx profiles leave it as
    /// retained engine state because wx has no equivalent public API.
    OnFocusChanged { focused: bool },

    // ---- Audio Events ----
    /// Notify that audio playback has been interrupted.
    ///
    /// Triggered when the system takes audio focus (e.g., incoming call).
    OnAudioInterruptionBegin,

    /// Notify that audio interruption has ended.
    ///
    /// Triggered when the system returns audio focus.
    OnAudioInterruptionEnd,

    /// InnerAudioContext event pushed from audio thread.
    ///
    /// Used to notify the JS layer of audio playback state changes.
    InnerAudioEvent {
        /// The InnerAudioContext instance ID.
        id: u32,
        /// Type of event (play, pause, ended, etc.).
        event_type: InnerAudioEventType,
        /// Current playback position in seconds.
        current_time: f64,
    },

    // ---- Rendering / Surface ----
    /// Update the rendering surface (e.g., after orientation change).
    ///
    /// The render thread will recreate the EGL context with the new surface.
    UpdateSurface {
        /// Generation-tagged Surface lease from the platform layer.
        lease: SurfaceLease,
        /// Host-authoritative DPR update. `None` preserves the current ratio.
        pixel_ratio: Option<PixelRatio>,
    },

    /// Notify that the current rendering surface has been destroyed.
    SurfaceDestroyed {
        /// Exact retired generation; delayed commands cannot affect a newer one.
        generation: SurfaceGeneration,
    },

    /// The renderer retired a still-live Surface after an unrecoverable native
    /// presentation failure. Expected host detach never emits this command.
    SurfaceLost {
        public_generation: PublicSurfaceGeneration,
        reason: SurfaceLossReason,
    },

    // ---- Touch / Input ----
    /// Dispatch touch input events to the game.
    ///
    /// Stored in a bounded, preallocated payload slot. Up to 512 normal
    /// commands can be pending, so enum size directly affects queue memory.
    OnTouch(Pooled<TouchData>),

    // ---- Sensor Events ----
    /// Device motion sensor data (rotation angles from TYPE_ROTATION_VECTOR).
    ///
    /// Sent by the platform sensor listener at the requested interval.
    /// Values follow the W3C DeviceOrientation spec:
    /// alpha = rotation around Z (0-360), beta = X (-180..180), gamma = Y (-90..90).
    OnDeviceMotionChange { alpha: f64, beta: f64, gamma: f64 },

    /// Gyroscope sensor data (angular velocity in rad/s).
    ///
    /// Sent by the platform gyroscope listener at the requested interval.
    OnGyroscopeChange { x: f64, y: f64, z: f64 },

    /// Device screen orientation changed (portrait/landscape).
    ///
    /// Sent by the platform when the display orientation changes.
    OnDeviceOrientationChange {
        /// One of: “portrait”, “landscape”, “landscapeReverse”.
        value: Cow<'static, str>,
    },

    /// Compass data (direction and accuracy).
    ///
    /// Sent by the platform compass listener (~5 times/second).
    OnCompassChange {
        /// Direction in degrees (0-360, 0 = north).
        direction: f64,
        /// Accuracy string (Android: “high”/”medium”/”low”/”no-contact”/”unreliable”).
        accuracy: Cow<'static, str>,
    },

    /// Accelerometer data (acceleration in m/s^2).
    ///
    /// Sent by the platform accelerometer listener at the requested interval.
    OnAccelerometerChange {
        /// Acceleration along X axis in m/s^2.
        x: f64,
        /// Acceleration along Y axis in m/s^2.
        y: f64,
        /// Acceleration along Z axis in m/s^2.
        z: f64,
    },

    // ---- Network Events ----
    /// Network status changed.
    ///
    /// Sent by the platform network monitor when connectivity changes.
    OnNetworkStatusChange {
        /// Whether network is connected.
        is_connected: bool,
        /// Network type: “wifi”, “2g”, “3g”, “4g”, “5g”, “unknown”, “none”.
        network_type: Cow<'static, str>,
    },

    // ---- Recorder Events ----
    /// Recorder event pushed from platform (start, pause, resume, stop, error, interruption).
    RecorderEvent {
        /// Event type string (e.g., “start”, “stop”, “error”, “interruptionBegin”).
        event_type: String,
        /// JSON-encoded payload (e.g., stop result with tempFilePath/duration/fileSize).
        json_payload: String,
    },

    /// Recorder frame data pushed from platform (for onFrameRecorded).
    RecorderFrameData {
        /// Raw PCM/encoded audio frame bytes.
        data: Vec<u8>,
        /// Whether this is the last frame before stop.
        is_last_frame: bool,
    },

    // ---- Camera Events ----
    /// Camera event pushed from platform (stop, authCancel, error, timeoutCallback).
    CameraEvent {
        /// JS-assigned camera instance ID.
        camera_id: u32,
        /// Event type string (e.g., “stop”, “authCancel”, “error”, “timeoutCallback”).
        event_type: String,
        /// JSON-encoded payload.
        json_payload: String,
    },

    /// Camera frame data pushed from platform (for onCameraFrame / listenFrameChange).
    CameraFrameData {
        /// JS-assigned camera instance ID.
        camera_id: u32,
        /// Raw Y/U/V plane-window bytes concatenated in Y, U, V order (each
        /// plane's `position..limit`), exactly as delivered to JS.
        data: Vec<u8>,
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
    },

    // ---- Keyboard Events ----
    /// Keyboard input event (user typed text in soft keyboard).
    ///
    /// Triggers `migo.onKeyboardInput` callbacks.
    OnKeyboardInput {
        /// Current text value of the keyboard input.
        value: String,
    },

    /// Keyboard height changed (soft keyboard shown/hidden or resized).
    ///
    /// Triggers `migo.onKeyboardHeightChange` callbacks.
    OnKeyboardHeightChange {
        /// Keyboard height in CSS pixels (0 when hidden).
        height: f64,
    },

    /// User pressed the confirm button on the soft keyboard.
    ///
    /// Triggers `migo.onKeyboardConfirm` callbacks.
    OnKeyboardConfirm {
        /// Current text value of the keyboard input.
        value: String,
    },

    /// Soft keyboard dismissed/completed.
    ///
    /// Triggers `migo.onKeyboardComplete` callbacks.
    OnKeyboardComplete {
        /// Current text value of the keyboard input.
        value: String,
    },

    /// Physical/PC keyboard key down event.
    ///
    /// Triggers `migo.onKeyDown` callbacks.
    OnKeyDown {
        /// Web KeyEvent.key value.
        key: String,
        /// Web KeyEvent.code value.
        code: String,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
        /// DOM modifier state as a bitmask. `key` alone cannot carry it: a
        /// modified press still reports the character it produces, so content
        /// could not tell `Ctrl+S` from `S`.
        modifiers: u32,
        /// DOM `KeyboardEvent.repeat`: the platform's auto-repeat produced this
        /// press, rather than the user pressing the key again.
        repeat: bool,
    },

    /// Physical/PC keyboard key up event.
    ///
    /// Triggers `migo.onKeyUp` callbacks.
    OnKeyUp {
        /// Web KeyEvent.key value.
        key: String,
        /// Web KeyEvent.code value.
        code: String,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
        /// DOM modifier state as a bitmask.
        modifiers: u32,
        /// DOM `KeyboardEvent.repeat`. Always false for a release on every
        /// platform that reports one, but carried so the two commands keep the
        /// same shape.
        repeat: bool,
    },

    // ---- Desktop pointer ----
    /// Mouse button pressed.
    ///
    /// Triggers `migo.onMouseDown` callbacks. Distinct from `OnTouch`, and a
    /// host chooses which it sends: wx content written for a phone listens for
    /// touch, wx content written for PC WeChat listens for the mouse, and only
    /// the host knows which streams its content and its device call for. The
    /// engine synthesizes neither from the other.
    OnMouseDown {
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        x: f32,
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        y: f32,
        /// Which button, in DOM `MouseEvent.button` order (0 = primary).
        button: u32,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
    },

    /// Mouse moved.
    ///
    /// Triggers `migo.onMouseMove` callbacks.
    OnMouseMove {
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        x: f32,
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        y: f32,
        /// Which button is held, in DOM `MouseEvent.button` order.
        button: u32,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
    },

    /// Mouse button released.
    ///
    /// Triggers `migo.onMouseUp` callbacks.
    OnMouseUp {
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        x: f32,
        /// CSS pixels, the same logical coordinate space as `OnTouch`.
        y: f32,
        /// Which button, in DOM `MouseEvent.button` order (0 = primary).
        button: u32,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
    },

    /// Wheel or trackpad scroll.
    ///
    /// Triggers `migo.onWheel` callbacks. The wheel has no touch equivalent, so
    /// unlike the mouse buttons above there is no other stream that could carry
    /// it.
    OnWheel {
        /// Horizontal delta, in the unit `delta_mode` names.
        delta_x: f64,
        /// Vertical delta, in the unit `delta_mode` names.
        delta_y: f64,
        /// Depth delta, in the unit `delta_mode` names; 0.0 on most devices.
        delta_z: f64,
        /// DOM `WheelEvent.deltaMode`: 0 = pixel, 1 = line, 2 = page. Carried
        /// rather than normalized to pixels because converting a line-based
        /// delta needs the content's line height, which only the content knows.
        delta_mode: u32,
        /// Event timestamp in milliseconds.
        timestamp_ms: f64,
    },

    // ---- IME Composition Events ----
    /// The user began composing text through an IME.
    ///
    /// Triggers `compositionstart`. Composition is the in-progress state of IME
    /// input -- typing pinyin shows a preedit string before any of it is
    /// committed -- and it is distinct from the soft keyboard's
    /// `OnKeyboardInput`, which reports text that has already been committed. A
    /// game drawing its own text field needs both: the preedit to show what is
    /// being typed, and the committed value to store.
    OnCompositionStart {
        /// The preedit text at the moment composition began; usually empty.
        data: String,
    },

    /// The preedit text changed.
    ///
    /// Triggers `compositionupdate`. `data` is the whole current preedit
    /// string, not the delta -- the same rule the soft keyboard's value follows,
    /// and for the same reason: a host sending only what changed leaves content
    /// unable to reconstruct the rest.
    OnCompositionUpdate { data: String },

    /// Composition finished.
    ///
    /// Triggers `compositionend`. `data` is the committed text, which is empty
    /// when the user cancelled. Content that has been drawing the preedit must
    /// clear it on this event; the committed text arrives here and, for a host
    /// that also drives the soft keyboard, again as an input value.
    OnCompositionEnd { data: String },

    // ---- Gamepad Events ----
    /// A gamepad became available in `index`.
    ///
    /// Triggers a `gamepadconnected` event. The id is the device's own name,
    /// which content shows to a player and uses to recognise a known pad.
    OnGamepadConnected {
        /// The slot the pad occupies for as long as it stays connected.
        index: u32,
        /// Human-readable device name, as the Web API's `Gamepad.id`.
        id: String,
        /// `"standard"` when the host mapped the pad onto the standard layout,
        /// and empty when it did not -- content reads it to decide whether the
        /// button order can be trusted.
        mapping: String,
        /// How many axes and buttons this pad reports.
        ///
        /// Carried on connect rather than inferred from the first state sample
        /// because `getGamepads()` must return correctly sized arrays from the
        /// moment the `gamepadconnected` listener runs -- content commonly reads
        /// `buttons.length` there to decide which layout it is looking at.
        axis_count: u8,
        button_count: u8,
    },

    /// The gamepad in `index` went away.
    ///
    /// Triggers a `gamepaddisconnected` event and empties the slot, so content
    /// polling `getGamepads()` sees a hole rather than a stale pad.
    OnGamepadDisconnected { index: u32 },

    /// New axis and button values for a connected gamepad.
    ///
    /// The Web API is polled rather than evented: content calls
    /// `getGamepads()` each frame and reads whatever is current. So this
    /// updates stored state instead of dispatching, and a host sends it as
    /// often as it samples.
    OnGamepadState(Pooled<GamepadState>),

    // ---- Bluetooth / BLE Events ----
    /// Bluetooth adapter state changed (available/discovering).
    ///
    /// Triggers `migo.onBluetoothAdapterStateChange` callbacks.
    OnBluetoothAdapterStateChange {
        /// Whether Bluetooth adapter is available.
        available: bool,
        /// Whether Bluetooth adapter is discovering devices.
        discovering: bool,
    },

    /// New Bluetooth device(s) found during discovery.
    ///
    /// Triggers `migo.onBluetoothDeviceFound` callbacks.
    OnBluetoothDeviceFound {
        /// JSON-encoded array of discovered devices.
        devices_json: String,
    },

    /// BLE connection state changed (connected/disconnected).
    ///
    /// Triggers `migo.onBLEConnectionStateChange` callbacks.
    OnBLEConnectionStateChange {
        /// BLE device identifier.
        device_id: String,
        /// Whether the device is connected.
        connected: bool,
    },

    /// BLE characteristic value changed (notification/indication received).
    ///
    /// Boxed to keep the `HostCommand` enum small (~56-64 bytes instead of ~216).
    /// Triggers `migo.onBLECharacteristicValueChange` callbacks.
    OnBLECharacteristicValueChange(Box<BleCharacteristicData>),

    /// BLE MTU changed after negotiation.
    ///
    /// Triggers `migo.onBLEMTUChange` callbacks.
    OnBLEMTUChange {
        /// BLE device identifier.
        device_id: String,
        /// New MTU value.
        mtu: u32,
    },

    /// Beacon devices updated during discovery.
    ///
    /// Triggers `migo.onBeaconUpdate` callbacks.
    OnBeaconUpdate {
        /// JSON-encoded array of beacon devices.
        beacons_json: String,
    },

    /// Beacon service state changed.
    ///
    /// Triggers `migo.onBeaconServiceChange` callbacks.
    OnBeaconServiceChange {
        /// Whether beacon service is available.
        available: bool,
        /// Whether beacon service is currently discovering.
        discovering: bool,
    },

    // ---- Video Events ----
    /// Video player state change event.
    ///
    /// Uses a single variant with `event_type` to cover all video events:
    /// "play", "pause", "ended", "timeupdate", "waiting", "progress",
    /// "error", "fullscreenchange".
    ///
    /// Triggers the corresponding event listener on the Video instance
    /// identified by `video_id`.
    OnVideoStateChange {
        /// The video player instance ID.
        video_id: u32,
        /// Event type: "play", "pause", "ended", "timeupdate", "waiting",
        /// "progress", "error", "fullscreenchange".
        event_type: String,
        /// JSON-encoded event data (e.g. `{"currentTime":12.5}` for timeupdate,
        /// `{"errMsg":"..."}` for error, `{"fullScreen":true,"direction":0}` for
        /// fullscreenchange).
        data: String,
    },

    // ---- System Events ----
    /// Memory warning from the system.
    ///
    /// Triggered when Android sends `onTrimMemory` or `onLowMemory`.
    /// Triggers `migo.onMemoryWarning` callbacks in the game.
    OnMemoryWarning {
        /// Memory warning level (Android TRIM_MEMORY_* constants):
        /// 5 = RUNNING_MODERATE, 10 = RUNNING_LOW, 15 = RUNNING_CRITICAL.
        level: i32,
    },

    /// User took a screenshot (system screenshot button pressed).
    ///
    /// Triggers `migo.onUserCaptureScreen` callback in the game.
    OnUserCaptureScreen,

    /// Thermal status changed (ADPF, API 29+).
    /// Level 0=none, 1=light, 2=moderate, 3=severe, 4=critical, 5=emergency, 6=shutdown.
    OnThermalStatusChanged { status: i32 },

    // ---- Display Configuration ----
    /// Display refresh period in nanoseconds (e.g., 16666667 for 60Hz, 8333333 for 120Hz).
    /// Sent once at session start and when display mode changes.
    SetDisplayRefreshRate { period_nanos: i64 },

    // ---- Host Message Channel ----
    /// Message from game JS to host app via migo.sendToHost(type, payload).
    ///
    /// `json` is a `{"type":"...","payload":"..."}` envelope.
    /// The host thread forwards this to `PlatformServices::notify_host_message`,
    /// which calls `NativeExports.onHostMessage` via JNI outbound.
    SendToHost { json: String },
}

// Guard against future regressions — if a new variant re-inflates the enum,
// this assertion will fail at compile time.
const _: () = assert!(
    core::mem::size_of::<HostCommand>() <= 64,
    "HostCommand grew past 64 bytes; check for unboxed large variants"
);

/// Event types for InnerAudioContext.
///
/// These map to the Mini Program InnerAudioContext events:
/// - `onCanplay`, `onPlay`, `onPause`, `onStop`, `onEnded`
/// - `onSeeking`, `onSeeked`, `onTimeUpdate`, `onError`
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InnerAudioEventType {
    /// Audio is ready to play (enough data buffered).
    CanPlay,
    /// Playback has started.
    Play,
    /// Playback has paused.
    Pause,
    /// Playback has stopped (and reset to beginning).
    Stop,
    /// Playback reached the end of the audio.
    Ended,
    /// A seek operation has started.
    Seeking,
    /// A seek operation has completed.
    Seeked,
    /// Current playback time has updated (periodic).
    TimeUpdate,
    /// Playback is waiting for more data (streaming buffer underrun).
    Waiting,
    /// An error occurred during playback.
    Error,
}

impl InnerAudioEventType {
    /// Convert to JS-compatible camelCase string.
    ///
    /// Used when dispatching events to JavaScript callbacks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let event = InnerAudioEventType::CanPlay;
    /// assert_eq!(event.as_str(), "canPlay");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            InnerAudioEventType::CanPlay => "canPlay",
            InnerAudioEventType::Play => "play",
            InnerAudioEventType::Pause => "pause",
            InnerAudioEventType::Stop => "stop",
            InnerAudioEventType::Ended => "ended",
            InnerAudioEventType::Seeking => "seeking",
            InnerAudioEventType::Seeked => "seeked",
            InnerAudioEventType::TimeUpdate => "timeUpdate",
            InnerAudioEventType::Waiting => "waiting",
            InnerAudioEventType::Error => "error",
        }
    }
}

/// A single touch point in a multi-touch event.
///
/// Must match the memory layout produced by the Java side `ByteBuffer`.
/// The layout is carefully designed for zero-copy transfer across JNI.
///
/// # Memory Layout
///
/// ```text
/// Offset  Field     Type    Size
/// ------  --------  ------  ----
///   0     id        u32     4
///   4     x         f32     4
///   8     y         f32     4
///  12     pressure  f32     4
///  16     flags     u32     4
/// ------  --------  ------  ----
/// Total: 20 bytes, aligned to 4 bytes
/// ```
///
/// # Fields
///
/// - `id`: Unique identifier for the touch pointer (stable across move events)
/// - `x`, `y`: Touch coordinates in CSS pixels (logical, not physical)
/// - `pressure`: Pressure value from 0.0 (none) to 1.0 (maximum)
/// - `flags`: Bitfield describing this pointer for the current event:
///   bit 0 (`0x1`) = in `changedTouches`; bit 1 (`0x2`) = removed from the
///   surface this event (finger up / cancel), so it is excluded from `touches`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct TouchPoint {
    /// Unique identifier for this touch pointer.
    /// Stable across the touch sequence (start → move → end).
    pub id: u32,
    /// X coordinate in CSS pixels (logical coordinates).
    pub x: f32,
    /// Y coordinate in CSS pixels (logical coordinates).
    pub y: f32,
    /// Touch pressure, normalized to 0.0–1.0 range.
    /// May be 0.0 if the device doesn't support pressure sensing.
    pub pressure: f32,
    /// Per-pointer bitfield: bit 0 = in `changedTouches`, bit 1 = removed from
    /// the surface this event (excluded from `touches`).
    pub flags: u32,
}

// Compile-time layout checks to prevent accidental ABI mismatch.
// These ensure the struct matches the Java/native side exactly.
// Must match Java's TOUCH_POINT_SIZE = 20 bytes.
const _: () = assert!(std::mem::size_of::<TouchPoint>() == 20);
const _: [(); 4] = [(); core::mem::align_of::<TouchPoint>()];

/// Type of touch event.
///
/// Maps to standard web/mobile touch event types.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TouchType {
    /// Touch started (finger down).
    Start = 0,
    /// Touch moved (finger dragged).
    Move = 1,
    /// Touch ended (finger up).
    End = 2,
    /// Touch cancelled (interrupted by system).
    Cancel = 3,
}
