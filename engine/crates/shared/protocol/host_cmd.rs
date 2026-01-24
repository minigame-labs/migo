use crate::surface::SurfaceRef;
use smallvec::SmallVec;

/// Commands sent to the host runtime thread.
#[non_exhaustive]
#[derive(Debug)]
pub enum HostCommand {
    /// Evaluate an ES module at `dir/entry`.
    EvaluateModule {
        dir: String,
        entry: String,
    },

    /// Evaluate a JS snippet (non-module).
    EvalScript {
        source: String,
    },

    Shutdown,

    OnShow,
    OnHide,

    UpdateSurface {
        surface: SurfaceRef,
    },

    OnTouch {
        touch_type: TouchType,
        points: SmallVec<[TouchPoint; 8]>,
        timestamp_ms: i64,
    },

    /// RequestAnimationFrame callback with a `time` value (milliseconds).
    RequestAnimationFrame(f64),
}

/// Must match the memory layout produced by the Java side `ByteBuffer`.
/// Layout: u32 + f32 + f32 + f32 + u32 = 20 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct TouchPoint {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub flags: u32,
}

// Compile-time layout checks to prevent accidental ABI mismatch.
// - size: 20 bytes
// - align: 4 bytes
const _: [(); 20] = [(); core::mem::size_of::<TouchPoint>()];
const _: [(); 4] = [(); core::mem::align_of::<TouchPoint>()];

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TouchType {
    Start = 0,
    Move = 1,
    End = 2,
    Cancel = 3,
}
