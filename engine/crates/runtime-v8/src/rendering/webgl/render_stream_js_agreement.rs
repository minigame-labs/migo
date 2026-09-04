//! The JavaScript encoder and the Rust wire-format table describe one thing.
//!
//! These cases live here rather than beside the validator because they are
//! about *this crate's* JavaScript: the validator moved to `migo-frame-wire` so
//! the cross-process consumer could use it without linking a JavaScript engine,
//! and a test that reads `00_render_command_stream.js` would have dragged that
//! coupling along with it. The format is shared; the encoder that has to agree
//! with it is not.
//!
//! `include_str!` resolves against this file, so the encoder read here is the
//! one this crate actually bakes into the V8 snapshot.

#![cfg(test)]

use frame_wire::gl_stream::*;

mod js_agreement {
    use super::*;

    // ── JS/Rust constant contract ────────────────────────────────────────────

    /// Every opcode the wire-format table declares appears in the JavaScript
    /// encoder with the same number.
    ///
    /// Both tables are PARSED, not restated. This test used to hold a
    /// hand-written list of sixty-nine name and value pairs, and a list someone
    /// has to extend is a list that falls behind: an opcode added to the Rust
    /// table and forgotten here would have left the test passing on the
    /// sixty-nine it already knew.
    ///
    /// The WebContent producer has a third copy of this table and cannot be
    /// read from inside this crate; `scripts/test-render-opcode-agreement.sh`
    /// checks all three together.
    #[test]
    fn the_javascript_encoder_declares_every_opcode_the_rust_table_does() {
        const RUST: &str = include_str!("../../../../frame-wire/src/gl_stream.rs");
        const JS: &str = include_str!("00_render_command_stream.js");

        fn parse(text: &str, prefix: &str, suffix: &str) -> Vec<(String, u32)> {
            let mut found = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix(prefix) else {
                    continue;
                };
                let Some((name, value)) = rest.split_once(suffix) else {
                    continue;
                };
                if !name.starts_with("OP_") {
                    continue;
                }
                let value = value.trim().trim_end_matches(';');
                let Ok(number) = value.parse::<u32>() else {
                    continue;
                };
                found.push((name.to_string(), number));
            }
            found
        }

        let rust = parse(RUST, "pub const ", ": u32 = ");
        let js = parse(JS, "const ", " = ");

        assert!(
            rust.len() >= 60,
            "only {} opcodes parsed from the Rust table; the pattern no longer matches",
            rust.len()
        );
        assert_eq!(
            rust.len(),
            js.len(),
            "the Rust table declares {} opcodes, the JavaScript encoder {}",
            rust.len(),
            js.len()
        );

        for (name, value) in &rust {
            let found = js
                .iter()
                .find(|(js_name, _)| js_name == name)
                .unwrap_or_else(|| panic!("the JavaScript encoder declares no {name}"));
            assert_eq!(
                found.1, *value,
                "{name} is {} in JavaScript and {value} in Rust",
                found.1
            );
        }
    }

    #[test]
    fn js_module_buffers_null_at_module_load() {
        let js = include_str!("00_render_command_stream.js");
        assert!(
            js.contains("= null;"),
            "JS module backing buffer vars must be initialized to null (lazy allocation)"
        );
    }

    /// No buffer references on globalThis.
    #[test]
    fn js_module_no_globalthis_assignment_of_buffers() {
        let js = include_str!("00_render_command_stream.js");
        assert!(
            !js.contains("globalThis."),
            "JS module must not assign buffers to globalThis"
        );
    }

    /// Hot encoders must not use rest params or temp words array.
    #[test]
    fn js_module_no_rest_params_in_encoders() {
        let js = include_str!("00_render_command_stream.js");
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
        let js = include_str!("00_render_command_stream.js");
        assert!(
            !js.contains("words = []"),
            "JS module must not allocate temporary words[] array in hot path"
        );
    }

    /// flushRenderCommandStream must pass used/cursor to op_submit_render_stream.
    #[test]
    fn js_module_flush_passes_cursor_to_op() {
        let js = include_str!("00_render_command_stream.js");
        assert!(
            js.contains("op_submit_render_stream"),
            "JS module must call op_submit_render_stream in flushRenderCommandStream"
        );
        assert!(
            js.contains("cursor"),
            "JS module flush must pass cursor (used_words) to op_submit_render_stream"
        );
    }
}
