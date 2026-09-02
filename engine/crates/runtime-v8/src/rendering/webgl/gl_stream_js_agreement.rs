//! The JavaScript encoder and the Rust wire-format table describe one thing.
//!
//! These cases live here rather than beside the validator because they are
//! about *this crate's* JavaScript: the validator moved to `migo-frame-wire` so
//! the cross-process consumer could use it without linking a JavaScript engine,
//! and a test that reads `00_gl_command_stream.js` would have dragged that
//! coupling along with it. The format is shared; the encoder that has to agree
//! with it is not.
//!
//! `include_str!` resolves against this file, so the encoder read here is the
//! one this crate actually bakes into the V8 snapshot.

#![cfg(test)]

use frame_wire::gl_stream::*;

mod js_agreement {
    use super::*;

    // ── JS/Rust constant contract: complete 69-opcode coverage ───────────────

    /// Asserts the JS stream module contains `const OP_<NAME> = <n>;` for every
    /// opcode constant defined in this file. This prevents silent divergence
    /// between the Rust wire-format table and the JS encoder table.
    #[test]
    fn js_module_contains_all_69_opcode_constants_matching_rust() {
        let js = include_str!("00_gl_command_stream.js");

        // Fixed opcodes 1..=58
        let fixed: &[(&str, u32)] = &[
            ("OP_VIEWPORT", OP_VIEWPORT),
            ("OP_CLEAR", OP_CLEAR),
            ("OP_CLEAR_COLOR", OP_CLEAR_COLOR),
            ("OP_CLEAR_DEPTH", OP_CLEAR_DEPTH),
            ("OP_CLEAR_STENCIL", OP_CLEAR_STENCIL),
            ("OP_ENABLE", OP_ENABLE),
            ("OP_DISABLE", OP_DISABLE),
            ("OP_USE_PROGRAM", OP_USE_PROGRAM),
            ("OP_BIND_BUFFER", OP_BIND_BUFFER),
            ("OP_BIND_TEXTURE", OP_BIND_TEXTURE),
            ("OP_ACTIVE_TEXTURE", OP_ACTIVE_TEXTURE),
            ("OP_BIND_FRAMEBUFFER", OP_BIND_FRAMEBUFFER),
            ("OP_BIND_RENDERBUFFER", OP_BIND_RENDERBUFFER),
            ("OP_BIND_VERTEX_ARRAY", OP_BIND_VERTEX_ARRAY),
            ("OP_BIND_SAMPLER", OP_BIND_SAMPLER),
            (
                "OP_ENABLE_VERTEX_ATTRIB_ARRAY",
                OP_ENABLE_VERTEX_ATTRIB_ARRAY,
            ),
            (
                "OP_DISABLE_VERTEX_ATTRIB_ARRAY",
                OP_DISABLE_VERTEX_ATTRIB_ARRAY,
            ),
            ("OP_VERTEX_ATTRIB_POINTER", OP_VERTEX_ATTRIB_POINTER),
            ("OP_VERTEX_ATTRIB_DIVISOR", OP_VERTEX_ATTRIB_DIVISOR),
            ("OP_BLEND_FUNC", OP_BLEND_FUNC),
            ("OP_BLEND_FUNC_SEPARATE", OP_BLEND_FUNC_SEPARATE),
            ("OP_BLEND_EQUATION", OP_BLEND_EQUATION),
            ("OP_BLEND_EQUATION_SEPARATE", OP_BLEND_EQUATION_SEPARATE),
            ("OP_BLEND_COLOR", OP_BLEND_COLOR),
            ("OP_DEPTH_FUNC", OP_DEPTH_FUNC),
            ("OP_DEPTH_MASK", OP_DEPTH_MASK),
            ("OP_DEPTH_RANGE", OP_DEPTH_RANGE),
            ("OP_STENCIL_FUNC", OP_STENCIL_FUNC),
            ("OP_STENCIL_FUNC_SEPARATE", OP_STENCIL_FUNC_SEPARATE),
            ("OP_STENCIL_OP", OP_STENCIL_OP),
            ("OP_STENCIL_OP_SEPARATE", OP_STENCIL_OP_SEPARATE),
            ("OP_STENCIL_MASK", OP_STENCIL_MASK),
            ("OP_STENCIL_MASK_SEPARATE", OP_STENCIL_MASK_SEPARATE),
            ("OP_CULL_FACE", OP_CULL_FACE),
            ("OP_FRONT_FACE", OP_FRONT_FACE),
            ("OP_COLOR_MASK", OP_COLOR_MASK),
            ("OP_SCISSOR", OP_SCISSOR),
            ("OP_LINE_WIDTH", OP_LINE_WIDTH),
            ("OP_POLYGON_OFFSET", OP_POLYGON_OFFSET),
            ("OP_TEX_PARAMETER_I", OP_TEX_PARAMETER_I),
            ("OP_TEX_PARAMETER_F", OP_TEX_PARAMETER_F),
            ("OP_GENERATE_MIPMAP", OP_GENERATE_MIPMAP),
            ("OP_PIXEL_STORE_I", OP_PIXEL_STORE_I),
            ("OP_HINT", OP_HINT),
            ("OP_SAMPLER_PARAMETER_I", OP_SAMPLER_PARAMETER_I),
            ("OP_SAMPLER_PARAMETER_F", OP_SAMPLER_PARAMETER_F),
            ("OP_DRAW_ARRAYS", OP_DRAW_ARRAYS),
            ("OP_DRAW_ELEMENTS", OP_DRAW_ELEMENTS),
            ("OP_DRAW_ARRAYS_INSTANCED", OP_DRAW_ARRAYS_INSTANCED),
            ("OP_DRAW_ELEMENTS_INSTANCED", OP_DRAW_ELEMENTS_INSTANCED),
            ("OP_BIND_BUFFER_BASE", OP_BIND_BUFFER_BASE),
            ("OP_BIND_BUFFER_RANGE", OP_BIND_BUFFER_RANGE),
            ("OP_READ_BUFFER", OP_READ_BUFFER),
            ("OP_UNIFORM1I", OP_UNIFORM1I),
            ("OP_UNIFORM1F", OP_UNIFORM1F),
            ("OP_UNIFORM2F", OP_UNIFORM2F),
            ("OP_UNIFORM3F", OP_UNIFORM3F),
            ("OP_UNIFORM4F", OP_UNIFORM4F),
        ];
        // Variable opcodes 256..=266
        let variable: &[(&str, u32)] = &[
            ("OP_UNIFORM1IV", OP_UNIFORM1IV),
            ("OP_UNIFORM1FV", OP_UNIFORM1FV),
            ("OP_UNIFORM2IV", OP_UNIFORM2IV),
            ("OP_UNIFORM2FV", OP_UNIFORM2FV),
            ("OP_UNIFORM3IV", OP_UNIFORM3IV),
            ("OP_UNIFORM3FV", OP_UNIFORM3FV),
            ("OP_UNIFORM4IV", OP_UNIFORM4IV),
            ("OP_UNIFORM4FV", OP_UNIFORM4FV),
            ("OP_UNIFORM_MATRIX2FV", OP_UNIFORM_MATRIX2FV),
            ("OP_UNIFORM_MATRIX3FV", OP_UNIFORM_MATRIX3FV),
            ("OP_UNIFORM_MATRIX4FV", OP_UNIFORM_MATRIX4FV),
        ];

        for &(name, value) in fixed.iter().chain(variable.iter()) {
            let expected = format!("const {} = {};", name, value);
            assert!(
                js.contains(&expected),
                "JS module missing '{}' (expected '{}' for Rust value {})",
                name,
                expected,
                value
            );
        }

        // Magic and version
        assert!(
            js.contains("const MAGIC = 0x4D474C31;"),
            "JS module missing 'const MAGIC = 0x4D474C31;'"
        );
        assert!(
            js.contains("const STREAM_VERSION = 1;"),
            "JS module missing 'const STREAM_VERSION = 1;'"
        );
        assert!(
            js.contains("const MAX_STREAM_UNIFORM_WORDS = 512;"),
            "JS module missing 'const MAX_STREAM_UNIFORM_WORDS = 512;'"
        );
    }

    // ── JS source-guard tests (host-runnable, via include_str!) ──────────────

    /// Buffers must be null at module load (lazy allocation).
    #[test]
    fn js_module_buffers_null_at_module_load() {
        let js = include_str!("00_gl_command_stream.js");
        assert!(
            js.contains("= null;"),
            "JS module backing buffer vars must be initialized to null (lazy allocation)"
        );
    }

    /// No buffer references on globalThis.
    #[test]
    fn js_module_no_globalthis_assignment_of_buffers() {
        let js = include_str!("00_gl_command_stream.js");
        assert!(
            !js.contains("globalThis."),
            "JS module must not assign buffers to globalThis"
        );
    }

    /// Hot encoders must not use rest params or temp words array.
    #[test]
    fn js_module_no_rest_params_in_encoders() {
        let js = include_str!("00_gl_command_stream.js");
        assert!(
            !js.contains("...args"),
            "JS module hot encoders must not use rest args (...args)"
        );
        assert!(
            !js.contains("encodeRecord("),
            "JS module must not have a generic encodeRecord(...args) dispatcher"
        );
    }

    /// No temporary words array allocation in hot path.
    #[test]
    fn js_module_no_temp_words_array_in_hot_path() {
        let js = include_str!("00_gl_command_stream.js");
        assert!(
            !js.contains("words = []"),
            "JS module must not allocate temporary words[] array in hot path"
        );
    }

    /// flushGlCommandStream must pass used/cursor to op_gl_submit_stream.
    #[test]
    fn js_module_flush_passes_cursor_to_op() {
        let js = include_str!("00_gl_command_stream.js");
        assert!(
            js.contains("op_gl_submit_stream"),
            "JS module must call op_gl_submit_stream in flushGlCommandStream"
        );
        assert!(
            js.contains("cursor"),
            "JS module flush must pass cursor (used_words) to op_gl_submit_stream"
        );
    }
}
