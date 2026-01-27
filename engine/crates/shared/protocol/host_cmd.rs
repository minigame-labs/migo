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
//! - **Rendering**: `RequestAnimationFrame`, `UpdateSurface`
//! - **Audio Events**: `InnerAudioEvent`

use crate::surface::SurfaceRef;
use smallvec::SmallVec;

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
///     points: smallvec![TouchPoint { id: 0, x: 100.0, y: 200.0, .. }],
///     timestamp_ms: 1234567890,
/// };
/// ```
#[non_exhaustive]
#[derive(Debug)]
pub enum HostCommand {
    /// Evaluate an ES module at `dir/entry`.
    ///
    /// This is the primary way to start a mini-game. The module is loaded
    /// relative to `dir`, and `entry` specifies the main module file.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// HostCommand::EvaluateModule {
    ///     dir: "/data/games/puzzle".to_string(),
    ///     entry: "game.js".to_string(),
    /// }
    /// ```
    EvaluateModule {
        /// Base directory containing the game code.
        dir: String,
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

    /// Update the rendering surface (e.g., after orientation change).
    ///
    /// The render thread will recreate the EGL context with the new surface.
    UpdateSurface {
        /// New surface reference from the platform layer.
        surface: SurfaceRef,
    },

    /// Dispatch touch input events to the game.
    ///
    /// Touch points are flattened from the platform's native format
    /// and delivered as a batch for efficiency.
    OnTouch {
        /// Type of touch event (start, move, end, cancel).
        touch_type: TouchType,
        /// Array of active touch points (optimized for ≤8 simultaneous touches).
        points: SmallVec<[TouchPoint; 8]>,
        /// Event timestamp in milliseconds (from system boot or epoch).
        timestamp_ms: i64,
    },

    /// RequestAnimationFrame callback with a `time` value (milliseconds).
    ///
    /// Sent by the render thread at the target frame rate (usually 60 FPS).
    /// The timestamp is relative to the render thread's start time.
    RequestAnimationFrame(f64),

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
}

/// Event types for InnerAudioContext.
///
/// These map to the WeChat Mini Program InnerAudioContext events:
/// - `onCanplay`, `onPlay`, `onPause`, `onStop`, `onEnded`
/// - `onSeeking`, `onSeeked`, `onTimeUpdate`, `onError`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
