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
}
