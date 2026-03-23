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
//! ## Command Categories (37 variants)
//!
//! - **Module Loading** (2): `EvaluateModule`, `EvalScript`
//! - **Lifecycle** (4): `Restart`, `Shutdown`, `OnShow`, `OnHide`
//! - **Audio** (3): `OnAudioInterruptionBegin`, `OnAudioInterruptionEnd`, `InnerAudioEvent`
//! - **Rendering / Surface** (1): `UpdateSurface`
//! - **Touch / Input** (1): `OnTouch`
//! - **Sensor Events** (5): `OnDeviceMotionChange` .. `OnAccelerometerChange`
//! - **Network** (1): `OnNetworkStatusChange`
//! - **Recorder** (2): `RecorderEvent`, `RecorderFrameData`
//! - **Camera** (2): `CameraEvent`, `CameraFrameData`
//! - **Keyboard** (6): `OnKeyboardInput` .. `OnKeyUp`
//! - **Bluetooth / BLE** (7): `OnBluetoothAdapterStateChange` .. `OnBeaconServiceChange`
//! - **Video** (1): `OnVideoStateChange`
//! - **System** (2): `OnMemoryWarning`, `OnUserCaptureScreen`

use crate::surface::SurfaceRef;

/// Commands sent to the host runtime thread.
///
/// These commands drive the JavaScript runtime and coordinate between
/// native subsystems (rendering, audio, input) and the JS game code.
///
/// # Variant Groups (37 variants total)
///
/// - **Module Loading** (2): `EvaluateModule`, `EvalScript`
/// - **Lifecycle** (4): `Restart`, `Shutdown`, `OnShow`, `OnHide`
/// - **Rendering / Surface** (1): `UpdateSurface`
/// - **Touch / Input** (1): `OnTouch`
/// - **Keyboard Events** (6): `OnKeyboardInput` .. `OnKeyUp`
/// - **Sensor Events** (5): `OnDeviceMotionChange` .. `OnAccelerometerChange`
/// - **Network** (1): `OnNetworkStatusChange`
/// - **Audio Events** (3): `OnAudioInterruptionBegin`, `OnAudioInterruptionEnd`, `InnerAudioEvent`
/// - **Recorder Events** (2): `RecorderEvent`, `RecorderFrameData`
/// - **Camera Events** (2): `CameraEvent`, `CameraFrameData`
/// - **Bluetooth / BLE Events** (7): `OnBluetoothAdapterStateChange` .. `OnBeaconServiceChange`
/// - **Video Events** (1): `OnVideoStateChange`
/// - **System Events** (2): `OnMemoryWarning`, `OnUserCaptureScreen`
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
/// let cmd = HostCommand::OnTouch {
///     touch_type: TouchType::Start,
///     count: 1,
///     points: Default::default(), // filled via ptr::copy_nonoverlapping
///     timestamp_ms: 1234567890,
/// };
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
        /// New surface reference from the platform layer.
        surface: SurfaceRef,
    },

    // ---- Touch / Input ----

    /// Dispatch touch input events to the game.
    ///
    /// Touch data is stored inline -- fixed `[TouchPoint; 10]` array with a count.
    /// No heap allocation, single memcpy from JNI DirectByteBuffer.
    OnTouch {
        /// Type of touch event (start, move, end, cancel).
        touch_type: TouchType,
        /// Number of valid touch points in the `points` array.
        count: u8,
        /// Fixed inline array of touch points (max 10 simultaneous).
        /// Only `points[..count]` are valid.
        points: [TouchPoint; 10],
        /// Event timestamp in milliseconds (from system boot or epoch).
        timestamp_ms: i64,
    },

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
        value: String,
    },

    /// Compass data (direction and accuracy).
    ///
    /// Sent by the platform compass listener (~5 times/second).
    OnCompassChange {
        /// Direction in degrees (0-360, 0 = north).
        direction: f64,
        /// Accuracy string (Android: “high”/”medium”/”low”/”no-contact”/”unreliable”).
        accuracy: String,
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
        network_type: String,
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
        /// Raw pixel data (RGBA).
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
    },

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
    /// Triggers `migo.onBLECharacteristicValueChange` callbacks.
    OnBLECharacteristicValueChange {
        /// BLE device identifier.
        device_id: String,
        /// GATT service UUID.
        service_id: String,
        /// GATT characteristic UUID.
        characteristic_id: String,
        /// Characteristic value bytes.
        value: Vec<u8>,
    },

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
}

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
/// - `flags`: Reserved for future use (e.g., touch type, device type)
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
    /// Reserved flags for future use.
    pub flags: u32,
}

// Compile-time layout checks to prevent accidental ABI mismatch.
// These ensure the struct matches the Java/native side exactly.
const _: [(); 20] = [(); core::mem::size_of::<TouchPoint>()];
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
