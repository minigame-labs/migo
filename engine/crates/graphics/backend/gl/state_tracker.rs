//! GL state-change deduplication predicates and update helpers.
//!
//! `StateTracker` is intentionally *stateless* — it takes a mutable
//! reference to the per-canvas [`crate::canvas::CanvasGLState`] and
//! reports "is this GL call redundant?" / records the new value after
//! the driver call.  Keeping this GL-free makes the dedup logic unit-
//! testable without an EGL context (see `tests` module at the bottom)
//! and also prevents the tracker from diverging from the actual GL
//! state — every mutation goes through one of the helpers below.
//!
//! Pattern used by the WebGL handler:
//!
//! ```ignore
//! if st::update_use_program(&mut cm.gl_state[cid], program_id) {
//!     unsafe { gl.use_program(Some(handle)) };
//! }
//! ```
//!
//! The `update_*` helper returns `true` when a real GL call must be
//! issued (the tracked value differs, or we have no prior value yet)
//! and `false` when the call is redundant.

use std::collections::HashSet;

use glow::{self};

use crate::canvas::{BlendEquation, BlendFactors, CanvasGLState, MAX_UNIFORM_CACHE};
use shared::protocol::render_cmd::{BufferId, ProgramId, VaoId};

// ============================================================================
// Program / uniforms
// ============================================================================

/// Returns `true` if `glUseProgram(new)` must actually be issued.
///
/// The tracker treats "unknown" (no previous program) as "must issue".
pub fn update_use_program(state: &mut CanvasGLState, new: ProgramId) -> bool {
    if state.current_program == Some(new) {
        return false;
    }
    state.current_program = Some(new);
    // Changing program invalidates our uniform cache.  Keep entries that
    // belong to the same program (user may re-use it later), drop ones
    // that belonged to whichever program came before.  The simpler
    // policy of flushing everything on switch is wrong for games that
    // cycle between two or three programs per frame.
    state.uniform_cache.retain(|(prog, _), _| *prog == new);
    true
}

/// Hash-and-compare dedup for a uniform upload.
///
/// `value_bytes` is the raw byte slice the caller would have handed to
/// `glUniform*` (e.g. `bytemuck::bytes_of(&[f32; 4])`).  Returns `true`
/// if the upload is not redundant and should be issued; on `true` the
/// cache entry is updated so the *next* call with identical bytes will
/// dedup.
pub fn update_uniform(
    state: &mut CanvasGLState,
    program: ProgramId,
    location: u32,
    value_bytes: &[u8],
) -> bool {
    let key = (program, location);
    match state.uniform_cache.get(&key) {
        Some(prev) if prev.as_ref() == value_bytes => return false,
        _ => {}
    }
    if state.uniform_cache.len() >= MAX_UNIFORM_CACHE && !state.uniform_cache.contains_key(&key) {
        // Simple LRU-ish eviction: drop an arbitrary entry.  A ring-buffer
        // would be better but this path triggers only for pathological
        // games using >256 unique locations per program.
        if let Some(k) = state.uniform_cache.keys().next().copied() {
            state.uniform_cache.remove(&k);
        }
    }
    state
        .uniform_cache
        .insert(key, Box::from(value_bytes));
    true
}

// ============================================================================
// Buffers + VAOs
// ============================================================================

/// Track `glBindBuffer(target, buf)`.  Currently dedups
/// `ARRAY_BUFFER` (`0x8892`) and `ELEMENT_ARRAY_BUFFER` (`0x8893`);
/// other targets (UNIFORM_BUFFER, PIXEL_UNPACK_BUFFER, …) always return
/// `true` until we grow explicit tracking for them.
pub fn update_bind_buffer(
    state: &mut CanvasGLState,
    target: u32,
    new: Option<BufferId>,
) -> bool {
    const GL_ARRAY_BUFFER: u32 = 0x8892;
    const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
    let new_opt = Some(new);
    match target {
        GL_ARRAY_BUFFER => {
            if state.bound_array_buffer == new_opt {
                return false;
            }
            state.bound_array_buffer = new_opt;
            true
        }
        GL_ELEMENT_ARRAY_BUFFER => {
            if state.bound_element_array_buffer == new_opt {
                return false;
            }
            state.bound_element_array_buffer = new_opt;
            true
        }
        _ => true,
    }
}

pub fn update_bind_vertex_array(state: &mut CanvasGLState, new: Option<VaoId>) -> bool {
    let new = new.unwrap_or(0);
    if state.bound_vao == Some(new) {
        return false;
    }
    state.bound_vao = Some(new);
    // Binding a new VAO changes the ARRAY_BUFFER binding semantically,
    // but the ELEMENT_ARRAY_BUFFER binding is stored *inside* the VAO —
    // forget our cached values to stay correct.
    state.bound_array_buffer = None;
    state.bound_element_array_buffer = None;
    true
}

// ============================================================================
// Textures
// ============================================================================

pub fn update_active_texture(state: &mut CanvasGLState, unit: u32) -> bool {
    if state.active_texture_unit == Some(unit) {
        return false;
    }
    state.active_texture_unit = Some(unit);
    true
}

/// Dedup `glBindTexture(TEXTURE_2D, tex)` scoped to the currently
/// active texture unit.  Callers without explicit active-unit tracking
/// must treat this as a pessimistic update (return `true`).
pub fn update_bind_texture_2d(state: &mut CanvasGLState, tex: Option<u32>) -> bool {
    let unit = match state.active_texture_unit {
        Some(u) => u,
        None => return true,
    };
    let entry = state.bound_texture_2d.entry(unit).or_insert(None);
    if *entry == tex {
        return false;
    }
    *entry = tex;
    true
}

// ============================================================================
// Viewport
// ============================================================================

/// Update the viewport fingerprint, returning `true` iff the driver
/// call must be issued.  Viewport is set on every frame by many
/// engines (often with the same values) — without dedup we pay an
/// unconditional GL round-trip per frame per canvas.
#[inline]
pub fn update_viewport(
    state: &mut CanvasGLState,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> bool {
    let new = (x, y, width, height);
    if state.viewport == Some(new) {
        return false;
    }
    state.viewport = Some(new);
    true
}

// ============================================================================
// Blend state
// ============================================================================

pub fn update_blend_func(state: &mut CanvasGLState, src: u32, dst: u32) -> bool {
    update_blend_func_separate(state, src, dst, src, dst)
}

pub fn update_blend_func_separate(
    state: &mut CanvasGLState,
    src_rgb: u32,
    dst_rgb: u32,
    src_alpha: u32,
    dst_alpha: u32,
) -> bool {
    let new = BlendFactors {
        src_rgb,
        dst_rgb,
        src_alpha,
        dst_alpha,
    };
    if state.blend_factors == Some(new) {
        return false;
    }
    state.blend_factors = Some(new);
    true
}

pub fn update_blend_equation(state: &mut CanvasGLState, mode: u32) -> bool {
    update_blend_equation_separate(state, mode, mode)
}

pub fn update_blend_equation_separate(
    state: &mut CanvasGLState,
    mode_rgb: u32,
    mode_alpha: u32,
) -> bool {
    let new = BlendEquation {
        mode_rgb,
        mode_alpha,
    };
    if state.blend_equation == Some(new) {
        return false;
    }
    state.blend_equation = Some(new);
    true
}

pub fn update_blend_color(state: &mut CanvasGLState, r: f32, g: f32, b: f32, a: f32) -> bool {
    let new = (r, g, b, a);
    if state.blend_color == Some(new) {
        return false;
    }
    state.blend_color = Some(new);
    true
}

// ============================================================================
// Depth / cull / front-face / misc scalars
// ============================================================================

pub fn update_depth_func(state: &mut CanvasGLState, func: u32) -> bool {
    if state.depth_func == Some(func) {
        return false;
    }
    state.depth_func = Some(func);
    true
}

pub fn update_depth_mask(state: &mut CanvasGLState, flag: bool) -> bool {
    if state.depth_mask == Some(flag) {
        return false;
    }
    state.depth_mask = Some(flag);
    true
}

pub fn update_depth_range(state: &mut CanvasGLState, near: f32, far: f32) -> bool {
    let new = (near, far);
    if state.depth_range == Some(new) {
        return false;
    }
    state.depth_range = Some(new);
    true
}

// ============================================================================
// Stencil state
// ============================================================================

/// Stencil-state dedup:  same (func, ref, mask) against the same face
/// is a no-op on the driver but a real round-trip without this.
/// `face` is `glow::FRONT`, `glow::BACK`, or `glow::FRONT_AND_BACK`;
/// we track per-face separately for the `_separate` calls and
/// duplicate the write when `FRONT_AND_BACK` is the face.
#[inline]
pub fn update_stencil_func(
    state: &mut CanvasGLState,
    face: u32,
    func: u32,
    ref_: i32,
    mask: u32,
) -> bool {
    let fp = (func, ref_, mask);
    let same = |k: u32| state.stencil_func.get(&k) == Some(&fp);
    if face == glow::FRONT_AND_BACK {
        if same(glow::FRONT) && same(glow::BACK) {
            return false;
        }
        state.stencil_func.insert(glow::FRONT, fp);
        state.stencil_func.insert(glow::BACK, fp);
    } else {
        if same(face) {
            return false;
        }
        state.stencil_func.insert(face, fp);
    }
    true
}

#[inline]
pub fn update_stencil_op(
    state: &mut CanvasGLState,
    face: u32,
    sfail: u32,
    dpfail: u32,
    dppass: u32,
) -> bool {
    let fp = (sfail, dpfail, dppass);
    let same = |k: u32| state.stencil_op.get(&k) == Some(&fp);
    if face == glow::FRONT_AND_BACK {
        if same(glow::FRONT) && same(glow::BACK) {
            return false;
        }
        state.stencil_op.insert(glow::FRONT, fp);
        state.stencil_op.insert(glow::BACK, fp);
    } else {
        if same(face) {
            return false;
        }
        state.stencil_op.insert(face, fp);
    }
    true
}

#[inline]
pub fn update_stencil_mask(state: &mut CanvasGLState, face: u32, mask: u32) -> bool {
    let same = |k: u32| state.stencil_mask.get(&k) == Some(&mask);
    if face == glow::FRONT_AND_BACK {
        if same(glow::FRONT) && same(glow::BACK) {
            return false;
        }
        state.stencil_mask.insert(glow::FRONT, mask);
        state.stencil_mask.insert(glow::BACK, mask);
    } else {
        if same(face) {
            return false;
        }
        state.stencil_mask.insert(face, mask);
    }
    true
}

// ============================================================================
// Pixel-storei
// ============================================================================

/// Many engines call `pixelStorei` repeatedly with identical `(pname,
/// param)` tuples between texture uploads.  A single HashMap keyed by
/// `pname` covers every pname the driver accepts; unknown pnames fall
/// back to "update and issue" every call (same as before).
#[inline]
pub fn update_pixel_store_i32(state: &mut CanvasGLState, pname: u32, param: i32) -> bool {
    if state.pixel_store_i32.get(&pname) == Some(&param) {
        return false;
    }
    state.pixel_store_i32.insert(pname, param);
    true
}

pub fn update_cull_face(state: &mut CanvasGLState, mode: u32) -> bool {
    if state.cull_face == Some(mode) {
        return false;
    }
    state.cull_face = Some(mode);
    true
}

pub fn update_front_face(state: &mut CanvasGLState, mode: u32) -> bool {
    if state.front_face == Some(mode) {
        return false;
    }
    state.front_face = Some(mode);
    true
}

pub fn update_line_width(state: &mut CanvasGLState, width: f32) -> bool {
    if state.line_width == Some(width) {
        return false;
    }
    state.line_width = Some(width);
    true
}

pub fn update_polygon_offset(state: &mut CanvasGLState, factor: f32, units: f32) -> bool {
    let new = (factor, units);
    if state.polygon_offset == Some(new) {
        return false;
    }
    state.polygon_offset = Some(new);
    true
}

pub fn update_unpack_alignment(state: &mut CanvasGLState, alignment: i32) -> bool {
    if state.unpack_alignment == Some(alignment) {
        return false;
    }
    state.unpack_alignment = Some(alignment);
    true
}

pub fn update_pack_alignment(state: &mut CanvasGLState, alignment: i32) -> bool {
    if state.pack_alignment == Some(alignment) {
        return false;
    }
    state.pack_alignment = Some(alignment);
    true
}

// ============================================================================
// Enable / disable capabilities
// ============================================================================

/// `glEnable(cap)` — returns true if a real call is required.
pub fn update_enable(state: &mut CanvasGLState, cap: u32) -> bool {
    if state.enabled_caps.contains(&cap) {
        return false;
    }
    state.enabled_caps.insert(cap);
    state.disabled_caps.remove(&cap);
    true
}

/// `glDisable(cap)` — returns true if a real call is required.
pub fn update_disable(state: &mut CanvasGLState, cap: u32) -> bool {
    if state.disabled_caps.contains(&cap) {
        return false;
    }
    state.disabled_caps.insert(cap);
    state.enabled_caps.remove(&cap);
    true
}

// ============================================================================
// Framebuffer / renderbuffer binding
// ============================================================================

/// `glBindFramebuffer(target, fb)`.  `fb = 0` (driver) == `None` shadow
/// (default FBO).  Returns `true` if the GL call must be issued.
///
/// WebGL spec: the same `target` value covers DRAW / READ on WebGL 1
/// (they're identical), but WebGL 2 separates them.  We shadow per
/// target key to keep both cases correct.
pub fn update_bind_framebuffer(
    state: &mut CanvasGLState,
    target: u32,
    fb: Option<u32>,
) -> bool {
    match state.bound_framebuffer.get(&target) {
        Some(shadow) if *shadow == fb => false,
        _ => {
            state.bound_framebuffer.insert(target, fb);
            true
        }
    }
}

/// `glBindRenderbuffer(RENDERBUFFER, rb)` dedup.  Only one target
/// (`GL_RENDERBUFFER`) exists in GLES; tracked with a single slot.
pub fn update_bind_renderbuffer(
    state: &mut CanvasGLState,
    rb: Option<u32>,
) -> bool {
    match state.bound_renderbuffer {
        Some(shadow) if shadow == rb => false,
        _ => {
            state.bound_renderbuffer = Some(rb);
            true
        }
    }
}

/// `glColorMask(r, g, b, a)`.
pub fn update_color_mask(
    state: &mut CanvasGLState,
    r: bool,
    g: bool,
    b: bool,
    a: bool,
) -> bool {
    let new = (r, g, b, a);
    if state.color_mask == new {
        return false;
    }
    state.color_mask = new;
    true
}

// ============================================================================
// Vertex attribute array state
// ============================================================================

/// `glEnableVertexAttribArray(index)`.  Returns `true` when the index
/// isn't already tracked as enabled.
pub fn update_enable_vertex_attrib(state: &mut CanvasGLState, index: u32) -> bool {
    if state.enabled_vertex_attribs.contains(&index) {
        return false;
    }
    state.enabled_vertex_attribs.insert(index);
    true
}

/// `glDisableVertexAttribArray(index)`.  Returns `true` when the index
/// was previously tracked as enabled.
pub fn update_disable_vertex_attrib(state: &mut CanvasGLState, index: u32) -> bool {
    if !state.enabled_vertex_attribs.contains(&index) {
        return false;
    }
    state.enabled_vertex_attribs.remove(&index);
    true
}

/// `glVertexAttribPointer(index, size, type, normalized, stride, offset)`.
///
/// WebGL's most-called non-draw call: Cocos Creator 2.x issues it 8+
/// times per sprite in the no-VAO code path.  Fingerprints the full
/// argument tuple PLUS the bound VAO PLUS the bound `ARRAY_BUFFER`
/// and skips the driver call only when every dimension matches the
/// previously cached entry for `(index)` (scoped by VAO + buffer in
/// the fingerprint).
///
/// Why `array_buffer` is in the fingerprint: WebGL 1.0 §5.14.10 and
/// GLES 3.0 §2.9.5 both say `vertexAttribPointer` captures the
/// currently bound `ARRAY_BUFFER` as the buffer source.  Skipping
/// a call just because `(size,type,...)` repeat — ignoring that
/// the current buffer is different — paints the next draw from the
/// WRONG vertex stream.  Static review caught this as a P0 bug.
pub fn update_vertex_attrib_pointer(
    state: &mut CanvasGLState,
    index: u32,
    size: i32,
    type_: u32,
    normalized: bool,
    stride: i32,
    offset: i32,
) -> bool {
    let vao = state.bound_vao.unwrap_or(0);
    let fp = crate::canvas::VertexAttribPointerFp {
        size,
        type_,
        normalized,
        stride,
        offset,
        vao,
        // The inner `Option<u32>` of `bound_array_buffer` is
        // `None` when no buffer has ever been bound (tracker
        // never saw a call) vs `Some(None)` for "known: no
        // buffer".  Both cases collapse to `None` here — we
        // conservatively force re-issue in the "never observed"
        // state by tracking None explicitly in the fingerprint.
        array_buffer: state.bound_array_buffer.and_then(|b| b),
    };
    let key = (vao, index);
    if state.vertex_attrib_pointer_fp.get(&key) == Some(&fp) {
        return false;
    }
    state.vertex_attrib_pointer_fp.insert(key, fp);
    true
}

/// `glVertexAttribDivisor(index, divisor)` dedup for WebGL 2 /
/// instanced_arrays.  A cheap keyed-shadow check.
pub fn update_vertex_attrib_divisor(
    state: &mut CanvasGLState,
    index: u32,
    divisor: u32,
) -> bool {
    match state.vertex_attrib_divisor.get(&index).copied() {
        Some(cur) if cur == divisor => false,
        _ => {
            state.vertex_attrib_divisor.insert(index, divisor);
            true
        }
    }
}

/// Test-only helper: construct a fresh state as the baseline for tests.
#[cfg(test)]
pub fn fresh_state() -> CanvasGLState {
    CanvasGLState::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Program dedup
    // ---------------------------------------------------------------------
    #[test]
    fn use_program_first_call_issues() {
        let mut s = fresh_state();
        assert!(update_use_program(&mut s, 7));
    }

    #[test]
    fn use_program_same_id_deduped() {
        let mut s = fresh_state();
        assert!(update_use_program(&mut s, 7));
        assert!(!update_use_program(&mut s, 7));
        assert!(!update_use_program(&mut s, 7));
    }

    #[test]
    fn use_program_different_id_reissues() {
        let mut s = fresh_state();
        assert!(update_use_program(&mut s, 1));
        assert!(update_use_program(&mut s, 2));
        assert!(!update_use_program(&mut s, 2));
        assert!(update_use_program(&mut s, 1));
    }

    #[test]
    fn use_program_flushes_other_programs_uniforms() {
        // Only entries for the new program should survive.
        let mut s = fresh_state();
        update_use_program(&mut s, 1);
        update_uniform(&mut s, 1, 10, &[1u8, 2, 3]);
        update_uniform(&mut s, 1, 20, &[4u8, 5]);
        update_use_program(&mut s, 2);
        update_uniform(&mut s, 2, 10, &[7u8]);
        // Switch back to 1; its uniform cache is now gone.
        update_use_program(&mut s, 1);
        // (1, 10) is gone ⇒ next upload must re-issue.
        assert!(update_uniform(&mut s, 1, 10, &[1u8, 2, 3]));
    }

    // ---------------------------------------------------------------------
    // Uniform dedup
    // ---------------------------------------------------------------------
    #[test]
    fn uniform_identical_bytes_dedup() {
        let mut s = fresh_state();
        let data = [0u8, 1, 2, 3];
        assert!(update_uniform(&mut s, 1, 5, &data));
        assert!(!update_uniform(&mut s, 1, 5, &data));
    }

    #[test]
    fn uniform_different_bytes_reissue() {
        let mut s = fresh_state();
        assert!(update_uniform(&mut s, 1, 5, &[1u8, 2]));
        assert!(update_uniform(&mut s, 1, 5, &[1u8, 3]));
    }

    #[test]
    fn uniform_different_locations_tracked_independently() {
        let mut s = fresh_state();
        assert!(update_uniform(&mut s, 1, 5, &[1u8]));
        assert!(update_uniform(&mut s, 1, 6, &[1u8]));
        assert!(!update_uniform(&mut s, 1, 5, &[1u8]));
        assert!(!update_uniform(&mut s, 1, 6, &[1u8]));
    }

    #[test]
    fn uniform_different_programs_tracked_independently() {
        let mut s = fresh_state();
        assert!(update_uniform(&mut s, 1, 5, &[9u8]));
        assert!(update_uniform(&mut s, 2, 5, &[9u8]));
        assert!(!update_uniform(&mut s, 1, 5, &[9u8]));
    }

    #[test]
    fn uniform_cache_bounded_by_max_entries() {
        let mut s = fresh_state();
        for i in 0..(MAX_UNIFORM_CACHE * 2) {
            update_uniform(&mut s, 1, i as u32, &[0u8]);
        }
        assert!(s.uniform_cache.len() <= MAX_UNIFORM_CACHE);
    }

    // ---------------------------------------------------------------------
    // Buffers
    // ---------------------------------------------------------------------
    #[test]
    fn bind_buffer_array_dedup() {
        let mut s = fresh_state();
        assert!(update_bind_buffer(&mut s, 0x8892, Some(1)));
        assert!(!update_bind_buffer(&mut s, 0x8892, Some(1)));
        assert!(update_bind_buffer(&mut s, 0x8892, Some(2)));
    }

    #[test]
    fn bind_buffer_unbind_tracked() {
        let mut s = fresh_state();
        update_bind_buffer(&mut s, 0x8892, Some(1));
        assert!(update_bind_buffer(&mut s, 0x8892, None));
        assert!(!update_bind_buffer(&mut s, 0x8892, None));
    }

    #[test]
    fn bind_buffer_other_targets_not_deduped() {
        // UNIFORM_BUFFER (0x8A11) not tracked yet → always issue.
        let mut s = fresh_state();
        assert!(update_bind_buffer(&mut s, 0x8A11, Some(1)));
        assert!(update_bind_buffer(&mut s, 0x8A11, Some(1)));
    }

    #[test]
    fn bind_vertex_array_forgets_buffer_bindings() {
        let mut s = fresh_state();
        update_bind_buffer(&mut s, 0x8892, Some(1));
        assert!(update_bind_vertex_array(&mut s, Some(5)));
        // After VAO change, ARRAY_BUFFER is unknown; next bind must issue.
        assert!(update_bind_buffer(&mut s, 0x8892, Some(1)));
    }

    // ---------------------------------------------------------------------
    // Textures
    // ---------------------------------------------------------------------
    #[test]
    fn active_texture_dedup() {
        let mut s = fresh_state();
        assert!(update_active_texture(&mut s, 0x84C0));
        assert!(!update_active_texture(&mut s, 0x84C0));
    }

    #[test]
    fn bind_texture_scoped_to_active_unit() {
        let mut s = fresh_state();
        update_active_texture(&mut s, 0x84C0);
        assert!(update_bind_texture_2d(&mut s, Some(1)));
        assert!(!update_bind_texture_2d(&mut s, Some(1)));
        // Switch unit; now binding 1 on a different unit must still issue.
        update_active_texture(&mut s, 0x84C1);
        assert!(update_bind_texture_2d(&mut s, Some(1)));
    }

    #[test]
    fn bind_texture_without_active_unit_pessimistic() {
        // If we never tracked an active unit, we can't safely dedup.
        let mut s = fresh_state();
        assert!(update_bind_texture_2d(&mut s, Some(1)));
        assert!(update_bind_texture_2d(&mut s, Some(1)));
    }

    // ---------------------------------------------------------------------
    // Blend / depth / cull / …
    // ---------------------------------------------------------------------
    #[test]
    fn blend_func_dedup() {
        let mut s = fresh_state();
        assert!(update_blend_func(&mut s, 1, 2));
        assert!(!update_blend_func(&mut s, 1, 2));
        assert!(update_blend_func(&mut s, 1, 3));
    }

    #[test]
    fn blend_func_separate_differs_from_plain_blend_func() {
        let mut s = fresh_state();
        update_blend_func(&mut s, 1, 2);
        // separate(1,2,1,2) matches plain blend_func; separate(1,2,3,4) doesn't.
        assert!(!update_blend_func_separate(&mut s, 1, 2, 1, 2));
        assert!(update_blend_func_separate(&mut s, 1, 2, 3, 4));
    }

    #[test]
    fn blend_equation_dedup() {
        let mut s = fresh_state();
        assert!(update_blend_equation(&mut s, 0x8006));
        assert!(!update_blend_equation(&mut s, 0x8006));
    }

    #[test]
    fn depth_func_dedup() {
        let mut s = fresh_state();
        assert!(update_depth_func(&mut s, 0x0203));
        assert!(!update_depth_func(&mut s, 0x0203));
        assert!(update_depth_func(&mut s, 0x0201));
    }

    #[test]
    fn depth_mask_boolean_round_trip() {
        let mut s = fresh_state();
        assert!(update_depth_mask(&mut s, true));
        assert!(!update_depth_mask(&mut s, true));
        assert!(update_depth_mask(&mut s, false));
    }

    #[test]
    fn cull_and_front_face_tracked_separately() {
        let mut s = fresh_state();
        assert!(update_cull_face(&mut s, 0x0405));
        assert!(!update_cull_face(&mut s, 0x0405));
        // front_face change must not affect cull_face cache.
        assert!(update_front_face(&mut s, 0x0900));
        assert!(!update_cull_face(&mut s, 0x0405));
    }

    // ---------------------------------------------------------------------
    // Enable / disable
    // ---------------------------------------------------------------------
    #[test]
    fn enable_then_enable_deduped() {
        let mut s = fresh_state();
        assert!(update_enable(&mut s, 0x0B71));
        assert!(!update_enable(&mut s, 0x0B71));
    }

    #[test]
    fn disable_after_enable_emits_once() {
        let mut s = fresh_state();
        update_enable(&mut s, 0x0B71);
        assert!(update_disable(&mut s, 0x0B71));
        assert!(!update_disable(&mut s, 0x0B71));
    }

    #[test]
    fn re_enable_after_disable_emits() {
        let mut s = fresh_state();
        update_enable(&mut s, 0x0B71);
        update_disable(&mut s, 0x0B71);
        assert!(update_enable(&mut s, 0x0B71));
    }

    // ---------------------------------------------------------------------
    // Cocos-shaped workload: simulate N draw calls that only differ in one
    // uniform, asserting the dedup ratio is >= 90%.  This is the "call
    // count" TDD guarantee promised by the Phase 8 plan.
    // ---------------------------------------------------------------------
    #[test]
    fn cocos_like_workload_deduplicates_most_calls() {
        let mut s = fresh_state();
        let mut issued = 0u32;

        // Typical Cocos 2.x inner loop over 100 sprites sharing one program:
        //   useProgram, bindBuffer(VBO), bindBuffer(EBO),
        //   bindTexture, activeTexture, uniform4f(u_color), drawElements.
        for i in 0..100 {
            if update_use_program(&mut s, 1) {
                issued += 1;
            }
            if update_bind_buffer(&mut s, 0x8892, Some(10)) {
                issued += 1;
            }
            if update_bind_buffer(&mut s, 0x8893, Some(11)) {
                issued += 1;
            }
            if update_active_texture(&mut s, 0x84C0) {
                issued += 1;
            }
            if update_bind_texture_2d(&mut s, Some(42)) {
                issued += 1;
            }
            // One uniform changes per iteration (e.g. the sprite's tint).
            let tint = [i as u8, 0, 0, 255];
            if update_uniform(&mut s, 1, 5, &tint) {
                issued += 1;
            }
        }

        // Of 600 possible redundant calls, only the very first iteration's
        // five state-setup calls are "real", plus 100 uniform uploads → 105.
        assert_eq!(
            issued, 105,
            "expected 105 real GL calls for the 600 logical calls, \
             got {issued} (dedup is broken)"
        );
    }

    // ---------------------------------------------------------------------
    // Sanity: fresh_state's extended dedup fields are all "unknown", so
    // every first setter call goes through.
    // ---------------------------------------------------------------------
    #[test]
    fn fresh_state_emits_first_call_for_every_setter() {
        let mut s = fresh_state();
        assert!(update_blend_func(&mut s, 1, 2));
        assert!(update_blend_equation(&mut s, 0x8006));
        assert!(update_blend_color(&mut s, 0.5, 0.5, 0.5, 1.0));
        assert!(update_depth_func(&mut s, 0x0203));
        assert!(update_depth_mask(&mut s, true));
        assert!(update_depth_range(&mut s, 0.0, 1.0));
        assert!(update_cull_face(&mut s, 0x0405));
        assert!(update_front_face(&mut s, 0x0900));
        assert!(update_line_width(&mut s, 1.0));
        assert!(update_polygon_offset(&mut s, 0.0, 0.0));
        assert!(update_unpack_alignment(&mut s, 4));
        assert!(update_pack_alignment(&mut s, 4));
    }

    // HashSet imported only so `enabled_caps`/`disabled_caps` don't warn.
    #[allow(dead_code)]
    fn _force_use_hashset() -> HashSet<u32> {
        HashSet::new()
    }

    // ---------------------------------------------------------------------
    // P11 dedup expansion — per-target framebuffer / renderbuffer,
    // vertex-attribute enable/pointer/divisor, and color mask.
    // ---------------------------------------------------------------------

    #[test]
    fn bind_framebuffer_first_call_issues_then_deduped() {
        let mut s = fresh_state();
        assert!(update_bind_framebuffer(&mut s, glow::FRAMEBUFFER, Some(7)));
        assert!(!update_bind_framebuffer(&mut s, glow::FRAMEBUFFER, Some(7)));
        // Different target is tracked independently — WebGL 2
        // separates DRAW / READ framebuffers, so this MUST reissue.
        assert!(update_bind_framebuffer(&mut s, glow::DRAW_FRAMEBUFFER, Some(7)));
        // Rebinding default FBO (0 / None) is a real call after a
        // named FBO was bound.
        assert!(update_bind_framebuffer(&mut s, glow::FRAMEBUFFER, None));
    }

    #[test]
    fn bind_renderbuffer_dedups_repeated_binds() {
        let mut s = fresh_state();
        assert!(update_bind_renderbuffer(&mut s, Some(3)));
        assert!(!update_bind_renderbuffer(&mut s, Some(3)));
        assert!(update_bind_renderbuffer(&mut s, Some(4)));
        assert!(update_bind_renderbuffer(&mut s, None));
    }

    #[test]
    fn color_mask_dedups_identical_tuples() {
        let mut s = fresh_state();
        // Initial state is (true,true,true,true); an identical call
        // must dedup on first touch.
        assert!(!update_color_mask(&mut s, true, true, true, true));
        assert!(update_color_mask(&mut s, false, false, false, false));
        assert!(!update_color_mask(&mut s, false, false, false, false));
    }

    #[test]
    fn enable_and_disable_vertex_attrib_are_idempotent() {
        let mut s = fresh_state();
        assert!(update_enable_vertex_attrib(&mut s, 0));
        assert!(!update_enable_vertex_attrib(&mut s, 0));
        assert!(update_disable_vertex_attrib(&mut s, 0));
        assert!(!update_disable_vertex_attrib(&mut s, 0));
        // Different index.
        assert!(update_enable_vertex_attrib(&mut s, 1));
        assert!(!update_enable_vertex_attrib(&mut s, 1));
    }

    #[test]
    fn vertex_attrib_pointer_dedups_identical_layout() {
        let mut s = fresh_state();
        // First call always issues.
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
        // Identical repeat — deduped.
        assert!(!update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
        // Different offset → re-issue.
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 16));
        // Different index → re-issue (tracked per-index).
        assert!(update_vertex_attrib_pointer(&mut s, 1, 4, glow::FLOAT, false, 32, 0));
    }

    #[test]
    fn vertex_attrib_pointer_reissues_after_vao_change() {
        // VAO capture semantics: the same (index, layout) after a
        // `bindVertexArray(2)` MUST hit the driver, even though the
        // layout tuple matches what was set for VAO 0.  If we ever
        // forget the VAO key, sprite batches switching VAOs silently
        // lose their vertex layout.
        let mut s = fresh_state();
        // Bind an ARRAY_BUFFER so the fingerprint has a concrete
        // buffer component — otherwise the two calls below also
        // differ in `array_buffer`, which would hide the VAO bug
        // behind the new buffer fingerprint.
        s.bound_array_buffer = Some(Some(99));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
        s.bound_vao = Some(2);
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
        assert!(!update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
    }

    // ---- ARRAY_BUFFER fingerprint (P0-3 regression) -----------------
    //
    // WebGL spec: `vertexAttribPointer` captures the currently-bound
    // `ARRAY_BUFFER`.  Two calls with identical (size, type, stride,
    // offset) but different bound buffers must both reach the driver
    // — otherwise the next draw reads from the wrong vertex stream
    // and the game paints corrupted geometry.
    //
    // These tests pin the fingerprint down so nobody accidentally
    // drops `array_buffer` again.

    #[test]
    fn vertex_attrib_pointer_reissues_when_array_buffer_changes() {
        let mut s = fresh_state();
        // Establish baseline with buffer A.
        s.bound_array_buffer = Some(Some(42));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
        // Switch ARRAY_BUFFER to buffer B — same layout args, but
        // a different buffer must force the driver call.
        s.bound_array_buffer = Some(Some(43));
        assert!(
            update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0),
            "switching ARRAY_BUFFER with identical pointer args MUST re-issue"
        );
        // Same buffer, same args — NOW the dedup should fire.
        assert!(!update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 32, 0));
    }

    #[test]
    fn vertex_attrib_pointer_reissues_when_buffer_goes_back_and_forth() {
        // Real Cocos 2.x workload: ping-pong between two VBOs
        // (positions vs. UVs) on the same attribute index across
        // sprite batches.  Each ping-pong must re-issue even
        // though the layout tuple is identical.
        let mut s = fresh_state();
        s.bound_array_buffer = Some(Some(1));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 2, glow::FLOAT, false, 8, 0));
        s.bound_array_buffer = Some(Some(2));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 2, glow::FLOAT, false, 8, 0));
        s.bound_array_buffer = Some(Some(1));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 2, glow::FLOAT, false, 8, 0));
        s.bound_array_buffer = Some(Some(1));
        assert!(!update_vertex_attrib_pointer(&mut s, 0, 2, glow::FLOAT, false, 8, 0));
    }

    #[test]
    fn vertex_attrib_pointer_keys_by_vao_scope_across_buffer_switches() {
        // Four distinct (VAO, ARRAY_BUFFER) combinations.  VAOs get
        // independent slots in the shadow; the `array_buffer`
        // component inside the fingerprint forces re-issue whenever
        // the bound buffer changes.  Re-visiting a *prior* buffer
        // on the same (VAO, index) re-issues — we don't retain
        // per-(VAO, index, buffer) history because Cocos-style
        // workloads overwhelmingly set the buffer once per VAO
        // scope and keep it, and the storage cost of retaining N
        // buffers per slot would dwarf the saved calls.
        //
        // Cross-VAO isolation IS preserved: switching to VAO=2 and
        // back to VAO=1 with the originally-bound buffer DOES
        // dedup, because VAO=1's slot was never touched while
        // VAO=2 was active.
        let mut s = fresh_state();
        s.bound_vao = Some(1);
        s.bound_array_buffer = Some(Some(10));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 16, 0));
        // Same VAO, different buffer → overwrites VAO 1's slot.
        s.bound_array_buffer = Some(Some(11));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 16, 0));
        // Switch to VAO 2, buffer 10.
        s.bound_vao = Some(2);
        s.bound_array_buffer = Some(Some(10));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 16, 0));
        // Switch back to VAO 1 WITHOUT touching its slot — the
        // previous (vao=1, buffer=11) fp is still cached, so
        // re-applying with buffer=11 dedups.
        s.bound_vao = Some(1);
        s.bound_array_buffer = Some(Some(11));
        assert!(!update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 16, 0));
        // But buffer=10 on VAO 1 was overwritten by the earlier
        // buffer=11 set, so it must re-issue now.
        s.bound_array_buffer = Some(Some(10));
        assert!(update_vertex_attrib_pointer(&mut s, 0, 4, glow::FLOAT, false, 16, 0));
    }

    #[test]
    fn vertex_attrib_divisor_dedups_per_index() {
        let mut s = fresh_state();
        assert!(update_vertex_attrib_divisor(&mut s, 0, 1));
        assert!(!update_vertex_attrib_divisor(&mut s, 0, 1));
        assert!(update_vertex_attrib_divisor(&mut s, 0, 0));
        assert!(update_vertex_attrib_divisor(&mut s, 1, 1));
    }

    #[test]
    fn stencil_func_dedup_per_face() {
        let mut s = fresh_state();
        assert!(update_stencil_func(
            &mut s,
            glow::FRONT,
            glow::EQUAL,
            0,
            0xFF
        ));
        assert!(!update_stencil_func(
            &mut s,
            glow::FRONT,
            glow::EQUAL,
            0,
            0xFF
        ));
        // BACK face has no fingerprint yet — must re-issue.
        assert!(update_stencil_func(
            &mut s,
            glow::BACK,
            glow::EQUAL,
            0,
            0xFF
        ));
        // FRONT_AND_BACK dedups only when BOTH faces already match.
        assert!(!update_stencil_func(
            &mut s,
            glow::FRONT_AND_BACK,
            glow::EQUAL,
            0,
            0xFF
        ));
    }

    #[test]
    fn stencil_op_front_and_back_dedup() {
        let mut s = fresh_state();
        assert!(update_stencil_op(
            &mut s,
            glow::FRONT_AND_BACK,
            glow::KEEP,
            glow::KEEP,
            glow::REPLACE
        ));
        // Identical FRONT_AND_BACK dedups.
        assert!(!update_stencil_op(
            &mut s,
            glow::FRONT_AND_BACK,
            glow::KEEP,
            glow::KEEP,
            glow::REPLACE
        ));
        // Single-face with identical fp dedups too.
        assert!(!update_stencil_op(
            &mut s,
            glow::FRONT,
            glow::KEEP,
            glow::KEEP,
            glow::REPLACE
        ));
    }

    #[test]
    fn stencil_mask_separate_front_back_tracked_independently() {
        let mut s = fresh_state();
        assert!(update_stencil_mask(&mut s, glow::FRONT, 0x0F));
        assert!(!update_stencil_mask(&mut s, glow::FRONT, 0x0F));
        assert!(update_stencil_mask(&mut s, glow::BACK, 0x0F));
        // FRONT_AND_BACK with matching values on both — dedups.
        assert!(!update_stencil_mask(&mut s, glow::FRONT_AND_BACK, 0x0F));
        // FRONT_AND_BACK with a new value — re-issues.
        assert!(update_stencil_mask(&mut s, glow::FRONT_AND_BACK, 0xF0));
    }

    #[test]
    fn pixel_store_i32_dedups_per_pname() {
        let mut s = fresh_state();
        assert!(update_pixel_store_i32(
            &mut s,
            glow::UNPACK_ALIGNMENT,
            4
        ));
        assert!(!update_pixel_store_i32(
            &mut s,
            glow::UNPACK_ALIGNMENT,
            4
        ));
        assert!(update_pixel_store_i32(
            &mut s,
            glow::UNPACK_ALIGNMENT,
            1
        ));
        // Different pname does not collide.
        assert!(update_pixel_store_i32(
            &mut s,
            glow::PACK_ALIGNMENT,
            1
        ));
    }

    #[test]
    fn depth_range_dedups_and_reissues_after_external() {
        let mut s = fresh_state();
        assert!(update_depth_range(&mut s, 0.0, 1.0));
        assert!(!update_depth_range(&mut s, 0.0, 1.0));
        assert!(update_depth_range(&mut s, 0.1, 0.9));
        s.invalidate_after_external_gl_use();
        assert!(update_depth_range(&mut s, 0.1, 0.9));
    }

    #[test]
    fn stencil_state_is_cleared_on_external_gl_use() {
        let mut s = fresh_state();
        let _ = update_stencil_func(&mut s, glow::FRONT, glow::EQUAL, 0, 0xFF);
        let _ = update_stencil_op(&mut s, glow::FRONT, glow::KEEP, glow::KEEP, glow::REPLACE);
        let _ = update_stencil_mask(&mut s, glow::FRONT, 0xFF);
        let _ = update_pixel_store_i32(&mut s, glow::UNPACK_ALIGNMENT, 4);
        s.invalidate_after_external_gl_use();
        // All four families re-issue on next call.
        assert!(update_stencil_func(&mut s, glow::FRONT, glow::EQUAL, 0, 0xFF));
        assert!(update_stencil_op(
            &mut s,
            glow::FRONT,
            glow::KEEP,
            glow::KEEP,
            glow::REPLACE
        ));
        assert!(update_stencil_mask(&mut s, glow::FRONT, 0xFF));
        assert!(update_pixel_store_i32(&mut s, glow::UNPACK_ALIGNMENT, 4));
    }

    #[test]
    fn viewport_dedups_identical_values() {
        let mut s = fresh_state();
        assert!(
            update_viewport(&mut s, 0, 0, 800, 600),
            "first call with unknown previous state must issue"
        );
        assert!(
            !update_viewport(&mut s, 0, 0, 800, 600),
            "identical viewport must dedup"
        );
        assert!(
            update_viewport(&mut s, 0, 0, 1024, 600),
            "width change must re-issue"
        );
        assert!(
            update_viewport(&mut s, 10, 0, 1024, 600),
            "origin change must re-issue"
        );
    }

    #[test]
    fn viewport_reissues_after_external_gl_use() {
        let mut s = fresh_state();
        let _ = update_viewport(&mut s, 0, 0, 800, 600);
        s.invalidate_after_external_gl_use();
        // After the Skia boundary, viewport may have been mutated
        // by Ganesh; the shadow is cleared so the next call
        // re-issues.
        assert!(update_viewport(&mut s, 0, 0, 800, 600));
    }

    #[test]
    fn invalidate_after_external_gl_use_preserves_uniform_cache() {
        // Regression from the 2026-04 rendering review: an earlier
        // revision called `self.uniform_cache.clear()` inside
        // `invalidate_after_external_gl_use`, contradicting the doc
        // comment and nuking the entire uniform dedup table every
        // Skia ↔ WebGL boundary crossing.  Uniforms live on GL
        // program objects, not on the shared context state Skia
        // touches, so the cache MUST survive.
        //
        // If this test ever fails, profile the next production
        // frame before re-adding the clear — it will re-issue every
        // `glUniform*` call the app already deduped, which is the
        // single dominant GL op category on shader-heavy workloads.
        let mut s = fresh_state();
        let prog: ProgramId = 7;
        let loc: u32 = 3;
        let bytes = [1u8, 2, 3, 4];
        assert!(update_uniform(&mut s, prog, loc, &bytes));
        // Second call with identical bytes — dedup must fire.
        assert!(!update_uniform(&mut s, prog, loc, &bytes));

        s.invalidate_after_external_gl_use();

        // After invalidation the uniform dedup entry MUST remain,
        // so the next identical call still dedups.
        assert!(
            !update_uniform(&mut s, prog, loc, &bytes),
            "uniform_cache was wiped by invalidate_after_external_gl_use"
        );
    }

    #[test]
    fn invalidate_after_external_gl_use_clears_p11_shadow() {
        let mut s = fresh_state();
        let _ = update_bind_framebuffer(&mut s, glow::FRAMEBUFFER, Some(7));
        let _ = update_enable_vertex_attrib(&mut s, 3);
        let _ = update_vertex_attrib_pointer(&mut s, 3, 4, glow::FLOAT, false, 32, 0);
        let _ = update_vertex_attrib_divisor(&mut s, 3, 1);

        s.invalidate_after_external_gl_use();

        // Every setter must re-issue after invalidation — that's the
        // whole point of marking state stale around a Skia batch.
        assert!(update_bind_framebuffer(&mut s, glow::FRAMEBUFFER, Some(7)));
        assert!(update_enable_vertex_attrib(&mut s, 3));
        assert!(update_vertex_attrib_pointer(&mut s, 3, 4, glow::FLOAT, false, 32, 0));
        assert!(update_vertex_attrib_divisor(&mut s, 3, 1));
    }
}
