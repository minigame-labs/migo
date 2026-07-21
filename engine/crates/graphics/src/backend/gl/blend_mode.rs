//! Canvas2D `globalCompositeOperation` ↔ Skia `BlendMode` mapping.
//!
//! The JS layer serialises [Canvas 2D spec compositing modes][spec] as a
//! stable `u8` code (see `_COMPOSITE_OPS` in
//! `engine/crates/runtime-v8/rendering/webgl/02_2d_context.js`).  This module
//! is the single source of truth translating that opcode into a
//! [`skia_safe::BlendMode`] at the render-thread boundary.
//!
//! Unknown / future codes fall back to `SrcOver` (the spec default) rather
//! than crashing — consistent with browser behaviour for unrecognised values.
//!
//! Notably:
//!   * `"lighter"`  → `BlendMode::Plus` (Skia calls additive blending "plus",
//!                    the spec calls it "lighter")
//!   * `"copy"`     → `BlendMode::Src`
//!
//! The 16 advanced non-separable / hue-chroma modes (`multiply`, `screen`,
//! `hue`, `saturation`, `color`, `luminosity`, etc.) are passed straight
//! through to Skia; the 11-entry legacy table used by femtovg is a strict
//! subset of what we expose here.
//!
//! [spec]: https://html.spec.whatwg.org/multipage/canvas.html#compositing

use skia_safe::BlendMode;

/// All 26 HTML Canvas 2D compositing operations, indexed by stable `u8` code.
///
/// Invariants verified by `tests::table_is_complete_and_ordered`:
///
/// * `TABLE[0]`  is the HTML default `"source-over"` → `BlendMode::SrcOver`
/// * indices `0..=10` match the legacy femtovg table exactly, so JS code that
///   predates the expansion still behaves correctly
/// * entries `11..=25` match the spec order of the 15 advanced modes
const TABLE: &[(&str, BlendMode); 26] = &[
    // Porter-Duff (11, indices 0..=10) -----------------------------------
    ("source-over", BlendMode::SrcOver),
    ("source-in", BlendMode::SrcIn),
    ("source-out", BlendMode::SrcOut),
    ("source-atop", BlendMode::SrcATop),
    ("destination-over", BlendMode::DstOver),
    ("destination-in", BlendMode::DstIn),
    ("destination-out", BlendMode::DstOut),
    ("destination-atop", BlendMode::DstATop),
    ("lighter", BlendMode::Plus),
    ("copy", BlendMode::Src),
    ("xor", BlendMode::Xor),
    // Advanced separable (11..=21) ---------------------------------------
    ("multiply", BlendMode::Multiply),
    ("screen", BlendMode::Screen),
    ("overlay", BlendMode::Overlay),
    ("darken", BlendMode::Darken),
    ("lighten", BlendMode::Lighten),
    ("color-dodge", BlendMode::ColorDodge),
    ("color-burn", BlendMode::ColorBurn),
    ("hard-light", BlendMode::HardLight),
    ("soft-light", BlendMode::SoftLight),
    ("difference", BlendMode::Difference),
    ("exclusion", BlendMode::Exclusion),
    // Non-separable / hue-chroma (22..=25) -------------------------------
    ("hue", BlendMode::Hue),
    ("saturation", BlendMode::Saturation),
    ("color", BlendMode::Color),
    ("luminosity", BlendMode::Luminosity),
];

/// Decode a compositing-operation opcode into a Skia [`BlendMode`].
///
/// Unknown codes return `BlendMode::SrcOver` — the spec default and the same
/// behaviour browsers exhibit when assigned an unrecognised string.
#[inline]
pub fn blend_mode_from_code(op: u8) -> BlendMode {
    TABLE
        .get(op as usize)
        .map(|(_, m)| *m)
        .unwrap_or(BlendMode::SrcOver)
}

/// Decode a compositing-operation opcode into its spec name.
/// Returns `"source-over"` for unknown codes.
///
/// Exposed primarily for tracing / debug tooling.
#[inline]
#[allow(dead_code)]
pub fn name_from_code(op: u8) -> &'static str {
    TABLE
        .get(op as usize)
        .map(|(n, _)| *n)
        .unwrap_or("source-over")
}

/// Number of valid compositing-operation codes (0..`OP_COUNT`).
pub const OP_COUNT: u8 = TABLE.len() as u8;

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::BlendMode::*;

    #[test]
    fn table_is_complete_and_ordered() {
        // The 26-entry table must stay in sync with the WHATWG Canvas 2D spec
        // section "compositing" order, AND indices 0..=10 must stay byte-for-
        // byte identical to the legacy 11-entry femtovg table so that JS
        // bytecode compiled against the old numbering continues to work.
        let names: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            &names[..11],
            &[
                "source-over",
                "source-in",
                "source-out",
                "source-atop",
                "destination-over",
                "destination-in",
                "destination-out",
                "destination-atop",
                "lighter",
                "copy",
                "xor",
            ]
        );
        assert_eq!(TABLE.len(), OP_COUNT as usize);
        assert_eq!(OP_COUNT, 26);
    }

    #[test]
    fn source_over_is_the_default() {
        assert_eq!(blend_mode_from_code(0), SrcOver);
        assert_eq!(name_from_code(0), "source-over");
    }

    #[test]
    fn lighter_maps_to_plus_not_screen() {
        // Canvas "lighter" is additive saturated blend (Skia `Plus`), NOT
        // the separable "screen" mode.  Regression check for a common bug.
        assert_eq!(blend_mode_from_code(8), Plus);
        assert_eq!(name_from_code(8), "lighter");
        assert_ne!(blend_mode_from_code(8), Screen);
    }

    #[test]
    fn copy_maps_to_src_not_clear() {
        assert_eq!(blend_mode_from_code(9), Src);
        assert_eq!(name_from_code(9), "copy");
    }

    #[test]
    fn porter_duff_modes_11_entries() {
        use BlendMode as B;
        let expected = [
            B::SrcOver,
            B::SrcIn,
            B::SrcOut,
            B::SrcATop,
            B::DstOver,
            B::DstIn,
            B::DstOut,
            B::DstATop,
            B::Plus,
            B::Src,
            B::Xor,
        ];
        for (op, mode) in expected.iter().enumerate() {
            assert_eq!(
                blend_mode_from_code(op as u8),
                *mode,
                "op code {op} ({}) mismatched",
                name_from_code(op as u8),
            );
        }
    }

    #[test]
    fn advanced_separable_modes_11_to_21() {
        use BlendMode as B;
        let expected = [
            (11, B::Multiply),
            (12, B::Screen),
            (13, B::Overlay),
            (14, B::Darken),
            (15, B::Lighten),
            (16, B::ColorDodge),
            (17, B::ColorBurn),
            (18, B::HardLight),
            (19, B::SoftLight),
            (20, B::Difference),
            (21, B::Exclusion),
        ];
        for (op, mode) in expected {
            assert_eq!(blend_mode_from_code(op), mode);
        }
    }

    #[test]
    fn non_separable_hue_chroma_modes_22_to_25() {
        use BlendMode as B;
        assert_eq!(blend_mode_from_code(22), B::Hue);
        assert_eq!(blend_mode_from_code(23), B::Saturation);
        assert_eq!(blend_mode_from_code(24), B::Color);
        assert_eq!(blend_mode_from_code(25), B::Luminosity);
    }

    #[test]
    fn unknown_opcodes_fall_back_to_source_over() {
        for op in [26u8, 27, 42, 100, 200, 255] {
            assert_eq!(
                blend_mode_from_code(op),
                SrcOver,
                "op={op} should fall back to SrcOver",
            );
            assert_eq!(name_from_code(op), "source-over");
        }
    }

    #[test]
    fn all_names_unique() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in TABLE.iter() {
            assert!(seen.insert(*n), "duplicate name {n}");
        }
    }

    #[test]
    fn legacy_11_entry_prefix_matches_femtovg_composite_operation_order() {
        // femtovg::CompositeOperation numeric order (from femtovg 0.22 source):
        //   0 SourceOver, 1 SourceIn, 2 SourceOut, 3 SourceAtop,
        //   4 DestinationOver, 5 DestinationIn, 6 DestinationOut,
        //   7 DestinationAtop, 8 Lighter, 9 Copy, 10 Xor
        // This test locks that prefix in; breaking it silently corrupts any
        // JS bytecode that was compiled assuming the old numbering.
        let legacy_names = [
            "source-over",
            "source-in",
            "source-out",
            "source-atop",
            "destination-over",
            "destination-in",
            "destination-out",
            "destination-atop",
            "lighter",
            "copy",
            "xor",
        ];
        for (op, expected_name) in legacy_names.iter().enumerate() {
            assert_eq!(name_from_code(op as u8), *expected_name);
        }
    }
}
