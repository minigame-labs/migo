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

/// The Canvas2D half: does each encoder write the record the reader will read?
///
/// The opcode gate next door checks that the three tables agree on *numbers*.
/// It cannot see the mistake that actually costs a frame, which is an encoder
/// that names the right opcode and writes the wrong number of words. Nothing
/// about that is a type error: the header carries the count the encoder claims,
/// the reader trusts it, and the next record starts in the middle of the
/// previous one. The whole rest of the buffer decodes as garbage or is refused,
/// on a device, with the frame not drawing.
///
/// The spec side is not parsed here -- `record_spec` is *called*, so this checks
/// the encoders against the same function the validator uses rather than against
/// a second reading of it.
mod canvas2d_agreement {
    use frame_wire::canvas2d::{OP2D_BASE, OP2D_END, OP2D_SELECT_CANVAS};
    use frame_wire::gl_stream::RecordSpec;
    use std::collections::HashMap;

    const JS: &str = include_str!("00_render_command_stream.js");

    /// `OP2D_NAME` -> value, from the JavaScript declarations.
    fn declared_opcodes() -> HashMap<String, u32> {
        let mut found = HashMap::new();
        for line in JS.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("const OP2D_") else {
                continue;
            };
            let Some((name, value)) = rest.split_once(" = ") else {
                continue;
            };
            let Ok(value) = value.trim().trim_end_matches(';').parse::<u32>() else {
                continue;
            };
            found.insert(format!("OP2D_{name}"), value);
        }
        found
    }

    /// The body of a `function <name>(...) { ... }` at column zero, brace-matched.
    fn function_body(name: &str) -> Option<&'static str> {
        let signature = format!("\nfunction {name}(");
        let start = JS.find(&signature)? + 1;
        let open = start + JS[start..].find('{')?;
        let bytes = JS.as_bytes();
        let mut depth = 0usize;
        for (offset, byte) in bytes[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&JS[open + 1..open + offset]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every `function encode2d*` / `_encode2d*` declared, in file order.
    fn encoder_names() -> Vec<String> {
        let mut names = Vec::new();
        for line in JS.lines() {
            let Some(rest) = line.strip_prefix("function ") else {
                continue;
            };
            let Some((name, _)) = rest.split_once('(') else {
                continue;
            };
            if name.starts_with("encode2d") || name.starts_with("_encode2d") {
                names.push(name.to_string());
            }
        }
        names
    }

    /// The word count a body claims: `packHeader(<anything>, N)` and
    /// `cursor = base + N` must be the same N, or the encoder writes one length
    /// into the header and advances by another -- which desynchronises the
    /// stream from the record *after* it, so the record that looks wrong is
    /// never the one that is.
    fn word_count_of(body: &str) -> u32 {
        let header = after(body, "packHeader(").expect("an encoder packs a header");
        let header = header
            .split_once(", ")
            .expect("packHeader takes an opcode and a count")
            .1;
        let claimed = leading_number(header).expect("the word count is a literal");

        let advance = after(body, "cursor = base + ").expect("an encoder advances the cursor");
        let advanced = leading_number(advance).expect("the cursor advance is a literal");

        assert_eq!(
            claimed, advanced,
            "an encoder writes a header claiming {claimed} words and advances {advanced}"
        );
        claimed
    }

    fn after<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
        text.find(needle).map(|at| &text[at + needle.len()..])
    }

    /// The digits this text starts with, if it starts with any.
    fn leading_number(text: &str) -> Option<u32> {
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    /// Each 2D opcode, and the encoder that writes it.
    fn encoders_by_opcode() -> HashMap<u32, (String, u32)> {
        let declared = declared_opcodes();
        // Shared bodies first: the one-line wrappers name a helper, not a header.
        let mut helper_words: HashMap<String, u32> = HashMap::new();
        for name in encoder_names() {
            if !name.starts_with('_') {
                continue;
            }
            let body = function_body(&name).expect("a declared function has a body");
            helper_words.insert(name, word_count_of(body));
        }

        let mut by_opcode = HashMap::new();
        for name in encoder_names() {
            if name.starts_with('_') {
                continue;
            }
            let body = function_body(&name).expect("a declared function has a body");
            // A wrapper delegates to a helper; anything else writes its own header.
            let delegated = helper_words.iter().find_map(|(helper, words)| {
                let call = format!("{helper}(OP2D_");
                let at = body.find(&call)?;
                let rest = &body[at + helper.len() + 1..];
                let opcode = rest.split(&[',', ')'][..]).next()?.trim();
                Some((declared.get(opcode).copied(), *words, opcode.to_string()))
            });

            let (opcode, words) = match delegated {
                Some((Some(opcode), words, _)) => (opcode, words),
                Some((None, _, opcode)) => panic!("{name} names {opcode}, which is not declared"),
                None => {
                    let header = after(body, "packHeader(")
                        .unwrap_or_else(|| panic!("{name} neither delegates nor packs a header"));
                    let opcode_name = header.split(',').next().expect("an opcode name").trim();
                    let opcode = *declared
                        .get(opcode_name)
                        .unwrap_or_else(|| panic!("{name} names undeclared {opcode_name}"));
                    (opcode, word_count_of(body))
                }
            };

            if let Some((other, _)) = by_opcode.insert(opcode, (name.clone(), words)) {
                panic!("opcode {opcode} is encoded by both {other} and {name}");
            }
        }
        by_opcode
    }

    #[test]
    fn every_encoder_writes_the_record_length_the_reader_expects() {
        let by_opcode = encoders_by_opcode();
        assert!(
            by_opcode.len() >= 30,
            "only {} encoders parsed out of the JavaScript; the pattern no longer matches",
            by_opcode.len()
        );

        for (opcode, (name, words)) in &by_opcode {
            let spec = frame_wire::canvas2d::record_spec(*opcode)
                .unwrap_or_else(|| panic!("{name} encodes opcode {opcode}, which has no spec"));
            let RecordSpec::Fixed { word_count, .. } = spec else {
                panic!("{name} encodes opcode {opcode}, which is not a fixed-length record");
            };
            assert_eq!(
                *words, word_count,
                "{name} writes {words} words for opcode {opcode}; the reader expects {word_count}"
            );
        }
    }

    #[test]
    fn every_2d_opcode_has_an_encoder() {
        let by_opcode = encoders_by_opcode();
        for opcode in OP2D_BASE..OP2D_END {
            if opcode == OP2D_SELECT_CANVAS {
                // Emitted by `begin2d`, not by an encoder of its own: content
                // never asks for it, the stream needs it whenever the reader
                // would not know which canvas the records belong to.
                assert!(
                    JS.contains("packHeader(OP2D_SELECT_CANVAS, 2)"),
                    "nothing emits the canvas selection, so every 2D record is one \
                     the reader refuses for not knowing where to draw"
                );
                continue;
            }
            assert!(
                by_opcode.contains_key(&opcode),
                "opcode {opcode} is in the wire table with no encoder; content \
                 reaching it falls back to an op crossing, silently"
            );
        }
    }

    /// The direction flags are real bools on the wire -- the validator refuses
    /// anything but 0 or 1 -- so the encoder has to narrow a truthy argument
    /// rather than pass it through.
    #[test]
    fn the_bool_words_are_narrowed_where_the_spec_says_they_are() {
        for (opcode, (name, _)) in encoders_by_opcode() {
            let Some(RecordSpec::Fixed { bool_words, .. }) =
                frame_wire::canvas2d::record_spec(opcode)
            else {
                continue;
            };
            let body = function_body(&name).expect("a declared function has a body");
            for index in bool_words {
                let write = format!("_u32[base + {index}] =");
                let at = body.find(&write).unwrap_or_else(|| {
                    panic!("{name} never writes word {index}, which the spec says is a bool")
                });
                let line = body[at..].lines().next().expect("a line");
                assert!(
                    line.contains("? 1 : 0"),
                    "{name} writes word {index} as `{}`; the spec says it is a bool and \
                     the validator refuses anything but 0 or 1",
                    line.trim()
                );
            }
        }
    }
}
