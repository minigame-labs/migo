//! Per-context WebGL error queue + cached context attributes.
//!
//! The Khronos WebGL 1.0 spec (§5.14.3) requires `getError()` to
//! return errors from a queue — the spec explicitly says
//! "implementations should track and return errors as they occur",
//! not "always return NO_ERROR".  Our previous stub returned `0`
//! unconditionally, which hid misuse bugs in games and misreported
//! WebGL conformance.  Firefox's `WebGLContextGL::GetError` maintains
//! a "webgl-side" error list that drains BEFORE consulting the
//! driver's `glGetError()`, so validation-stage errors don't get lost
//! behind a downstream driver error.
//!
//! We mirror that two-level design here, but only the host-side
//! queue is populated today: validation ops that detect an illegal
//! enum / operation / value before dispatch push the WebGL error
//! code into this queue; an optional driver-side drain (via a
//! future synchronous op against `gl.get_error()`) can be layered
//! on later without JS-facing changes.
//!
//! Error codes are raw u32 values matching the WebGL constants:
//! `INVALID_ENUM = 0x0500`, `INVALID_VALUE = 0x0501`,
//! `INVALID_OPERATION = 0x0502`, `OUT_OF_MEMORY = 0x0505`,
//! `INVALID_FRAMEBUFFER_OPERATION = 0x0506`, `CONTEXT_LOST_WEBGL = 0x9242`.

use std::collections::{HashMap, VecDeque};

use deno_core::OpState;

/// Pushed by host-side validators; drained by `op_get_error`.
///
/// Separate `HashMap<canvas_id, queue>` (instead of one global queue)
/// because each WebGL context has its own error queue in the spec:
/// `getError()` on `ctxA` must not return errors that originated on
/// `ctxB`.
#[derive(Default)]
pub struct WebGLErrorState {
    queues: HashMap<u32, VecDeque<u32>>,
    /// Cached context attributes per canvas.  Returned as-is by
    /// `getContextAttributes()`.
    attrs: HashMap<u32, ContextAttributes>,
}

/// Mirror of WebGLContextAttributes IDL dictionary.  Values are the
/// *actual* parameters the runtime chose, so JS gets truthful info
/// (not the defaults it might have requested).
#[derive(Clone, Copy, Debug)]
pub struct ContextAttributes {
    pub alpha: bool,
    pub antialias: bool,
    pub depth: bool,
    pub stencil: bool,
    pub premultiplied_alpha: bool,
    pub preserve_drawing_buffer: bool,
    /// `"default" | "high-performance" | "low-power"` — stored as a
    /// small enum to keep the op table lean.
    pub power_preference: PowerPreference,
    pub fail_if_major_performance_caveat: bool,
    pub desynchronized: bool,
    pub xr_compatible: bool,
}

impl Default for ContextAttributes {
    fn default() -> Self {
        // Defaults mirror the WebGL 1.0 spec (§5.2.1).
        Self {
            alpha: true,
            antialias: true,
            depth: true,
            stencil: false,
            premultiplied_alpha: true,
            preserve_drawing_buffer: false,
            power_preference: PowerPreference::Default,
            fail_if_major_performance_caveat: false,
            desynchronized: false,
            xr_compatible: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PowerPreference {
    Default,
    HighPerformance,
    LowPower,
}

impl PowerPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::HighPerformance => "high-performance",
            Self::LowPower => "low-power",
        }
    }
}

impl WebGLErrorState {
    /// Push a WebGL error code onto the queue for `canvas_id`.
    ///
    /// WebGL spec semantics: if a previous call already recorded the
    /// same error code, the spec allows us to coalesce, but Chrome /
    /// Firefox queue them separately; we match the latter so
    /// conformance scripts that count errors behave identically.
    pub fn push(&mut self, canvas_id: u32, code: u32) {
        self.queues.entry(canvas_id).or_default().push_back(code);
    }

    /// Drain the oldest error for `canvas_id`, or return
    /// `NO_ERROR (0)` when the queue is empty.
    pub fn drain_one(&mut self, canvas_id: u32) -> u32 {
        self.queues
            .get_mut(&canvas_id)
            .and_then(|q| q.pop_front())
            .unwrap_or(0)
    }

    /// Query current queue depth.  Used in tests; production code
    /// should use `drain_one` + comparison.
    #[allow(dead_code)]
    pub fn len(&self, canvas_id: u32) -> usize {
        self.queues.get(&canvas_id).map_or(0, |q| q.len())
    }

    pub fn set_attrs(&mut self, canvas_id: u32, attrs: ContextAttributes) {
        self.attrs.insert(canvas_id, attrs);
    }

    pub fn get_attrs(&self, canvas_id: u32) -> Option<ContextAttributes> {
        self.attrs.get(&canvas_id).copied()
    }
}

/// Convenience for host-side validators: push a WebGL error code
/// without needing to thread `WebGLErrorState` manually.
#[inline]
pub fn push_error(state: &mut OpState, canvas_id: u32, code: u32) {
    let q = state.borrow_mut::<WebGLErrorState>();
    q.push(canvas_id, code);
}

// ---- Ops ------------------------------------------------------------

/// `gl.getError()` — drain one pending error, or return `NO_ERROR`.
#[deno_core::op2(fast)]
pub fn op_webgl_get_error(state: &mut OpState, #[smi] canvas_id: u32) -> u32 {
    let q = state.borrow_mut::<WebGLErrorState>();
    q.drain_one(canvas_id)
}

/// Serializable mirror of `ContextAttributes` with camelCase field
/// names (to match the WebGLContextAttributes IDL dictionary).
#[derive(serde::Serialize)]
pub struct SerializedAttrs {
    pub alpha: bool,
    pub antialias: bool,
    pub depth: bool,
    pub stencil: bool,
    #[serde(rename = "premultipliedAlpha")]
    pub premultiplied_alpha: bool,
    #[serde(rename = "preserveDrawingBuffer")]
    pub preserve_drawing_buffer: bool,
    #[serde(rename = "powerPreference")]
    pub power_preference: &'static str,
    #[serde(rename = "failIfMajorPerformanceCaveat")]
    pub fail_if_major_performance_caveat: bool,
    pub desynchronized: bool,
    #[serde(rename = "xrCompatible")]
    pub xr_compatible: bool,
}

impl From<ContextAttributes> for SerializedAttrs {
    fn from(a: ContextAttributes) -> Self {
        Self {
            alpha: a.alpha,
            antialias: a.antialias,
            depth: a.depth,
            stencil: a.stencil,
            premultiplied_alpha: a.premultiplied_alpha,
            preserve_drawing_buffer: a.preserve_drawing_buffer,
            power_preference: a.power_preference.as_str(),
            fail_if_major_performance_caveat: a.fail_if_major_performance_caveat,
            desynchronized: a.desynchronized,
            xr_compatible: a.xr_compatible,
        }
    }
}

/// `gl.getContextAttributes()` - return cached actual attributes.
/// Never returns `null`; a fresh context that hasn't called
/// `set_attrs` yet gets the spec defaults.
#[deno_core::op2]
#[serde]
pub fn op_webgl_get_context_attributes(
    state: &mut OpState,
    #[smi] canvas_id: u32,
) -> SerializedAttrs {
    let q = state.borrow::<WebGLErrorState>();
    SerializedAttrs::from(q.get_attrs(canvas_id).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_queue_returns_no_error() {
        let mut q = WebGLErrorState::default();
        assert_eq!(q.drain_one(1), 0);
        assert_eq!(q.drain_one(99), 0); // unknown canvas
    }

    #[test]
    fn pushed_errors_drain_fifo() {
        let mut q = WebGLErrorState::default();
        q.push(1, 0x0500);
        q.push(1, 0x0501);
        q.push(1, 0x0502);
        assert_eq!(q.drain_one(1), 0x0500);
        assert_eq!(q.drain_one(1), 0x0501);
        assert_eq!(q.drain_one(1), 0x0502);
        assert_eq!(q.drain_one(1), 0); // drained
    }

    #[test]
    fn queues_are_scoped_per_canvas() {
        let mut q = WebGLErrorState::default();
        q.push(1, 0x0500);
        q.push(2, 0x0502);
        assert_eq!(q.drain_one(1), 0x0500);
        assert_eq!(q.drain_one(2), 0x0502);
        assert_eq!(q.drain_one(1), 0);
        assert_eq!(q.drain_one(2), 0);
    }

    #[test]
    fn default_attrs_match_spec() {
        let q = WebGLErrorState::default();
        let a = q.get_attrs(1).unwrap_or_default();
        assert!(a.alpha);
        assert!(a.antialias);
        assert!(a.depth);
        assert!(!a.stencil);
        assert!(a.premultiplied_alpha);
        assert!(!a.preserve_drawing_buffer);
        assert_eq!(a.power_preference.as_str(), "default");
    }

    #[test]
    fn set_attrs_round_trips() {
        let mut q = WebGLErrorState::default();
        let mut a = ContextAttributes::default();
        a.antialias = false;
        a.power_preference = PowerPreference::HighPerformance;
        q.set_attrs(1, a);
        let got = q.get_attrs(1).unwrap();
        assert!(!got.antialias);
        assert_eq!(got.power_preference.as_str(), "high-performance");
    }
}
