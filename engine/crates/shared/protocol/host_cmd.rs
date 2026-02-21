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
//! - **Module Loading**: `EvaluateModule`, `EvalScript`
//! - **Lifecycle**: `Shutdown`, `OnShow`, `OnHide`
//! - **Input**: `OnTouch`
//! - **Rendering**: `UpdateSurface`
//! - **Audio Events**: `InnerAudioEvent`

use crate::surface::SurfaceRef;

/// Commands sent to the host runtime thread.
///
/// These commands drive the JavaScript runtime and coordinate between
/// native subsystems (rendering, audio, input) and the JS game code.
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
///     dir: "/data/game".to_string(),
///     entry: "main.js".to_string(),
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
    /// Evaluate an ES module with isolated VFS paths.
    ///
    /// This is the primary way to start a mini-game. The module is loaded
    /// from the game's code directory, and sandboxed file access is enabled.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// HostCommand::EvaluateModule {
    ///     game_id: "my-puzzle-game".to_string(),
    ///     entry: "game.js".to_string(),
    /// }
    /// ```
    EvaluateModule {
        /// Unique game identifier (1-64 alphanumeric, underscore, hyphen).
        /// Used to derive all game paths from base directories.
        game_id: String,
        /// Entry point module filename (e.g., "main.js", "game.js").
        entry: String,
    },

    /// Evaluate a JS snippet (non-module).
    ///
    /// Useful for debugging or dynamic code execution. The code runs
    /// in the global scope, not as a module.
    EvalScript {
        /// JavaScript source code to execute.
        source: String,
    },

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
    OnShow,

    /// Notify that the app has become hidden/inactive.
    ///
    /// Triggers `migo.onHide` callbacks in the game.
    OnHide,

    /// Notify that audio playback has been interrupted.
    ///
    /// Triggered when the system takes audio focus (e.g., incoming call).
    OnAudioInterruptionBegin,

    /// Notify that audio interruption has ended.
    ///
    /// Triggered when the system returns audio focus.
    OnAudioInterruptionEnd,

    /// Update the rendering surface (e.g., after orientation change).
    ///
    /// The render thread will recreate the EGL context with the new surface.
    UpdateSurface {
        /// New surface reference from the platform layer.
        surface: SurfaceRef,
    },

    /// Dispatch touch input events to the game.
    ///
    /// Touch data is stored inline — fixed `[TouchPoint; 10]` array with a count.
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

    /// Device motion sensor data (rotation angles from TYPE_ROTATION_VECTOR).
    ///
    /// Sent by the platform sensor listener at the requested interval.
    /// Values follow the W3C DeviceOrientation spec:
    /// alpha = rotation around Z (0-360), beta = X (-180..180), gamma = Y (-90..90).
    OnDeviceMotionChange {
        alpha: f64,
        beta: f64,
        gamma: f64,
    },

    /// Gyroscope sensor data (angular velocity in rad/s).
    ///
    /// Sent by the platform gyroscope listener at the requested interval.
    OnGyroscopeChange {
        x: f64,
        y: f64,
        z: f64,
    },

    /// Device screen orientation changed (portrait/landscape).
    ///
    /// Sent by the platform when the display orientation changes.
    OnDeviceOrientationChange {
        /// One of: "portrait", "landscape", "landscapeReverse".
        value: String,
    },

    /// Compass data (direction and accuracy).
    ///
    /// Sent by the platform compass listener (~5 times/second).
    OnCompassChange {
        /// Direction in degrees (0-360, 0 = north).
        direction: f64,
        /// Accuracy string (Android: "high"/"medium"/"low"/"no-contact"/"unreliable").
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

    /// Network status changed.
    ///
    /// Sent by the platform network monitor when connectivity changes.
    OnNetworkStatusChange {
        /// Whether network is connected.
        is_connected: bool,
        /// Network type: "wifi", "2g", "3g", "4g", "5g", "unknown", "none".
        network_type: String,
    },

    /// Recorder event pushed from platform (start, pause, resume, stop, error, interruption).
    RecorderEvent {
        /// Event type string (e.g., "start", "stop", "error", "interruptionBegin").
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
