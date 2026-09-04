//! Emit ASTC blocks and the pixels they came from, for the decoder that is not
//! ours to check.
//!
//! `#[ignore]`d because a `cargo test` run has no way to *use* what it writes:
//! the check needs a GL stack with an ASTC decoder, which is a dependency a
//! Rust test cannot express. That is the visible form of the dependency;
//! `scripts/test-astc-encoder.sh` is what guarantees this actually runs, which
//! is the half a test returning early on a missing environment variable loses.

use std::path::PathBuf;

use migo_io::astc::{Footprint, encode_astc, encode_astc_within, worst_error};

/// The same budget `ingest_transcode` applies, because what this checks is the
/// choice that path makes.
const BUDGET: u8 = 8;

/// A fixture: a name, a size, and a function from position to RGBA.
struct Fixture {
    name: &'static str,
    width: u32,
    height: u32,
    pixel: fn(u32, u32) -> [u8; 4],
}

/// Deliberately chosen for what each one can break.
const FIXTURES: &[Fixture] = &[
    // A flat block exercises nothing but the endpoint path, which is where a
    // wrong quantisation table shows up undiluted.
    Fixture {
        name: "flat",
        width: 24,
        height: 24,
        pixel: |_, _| [200, 100, 50, 255],
    },
    // Colour varying along x while alpha varies along y is the case single-plane
    // ASTC cannot represent: if the planes were not independent, one of the two
    // gradients would come back flat.
    Fixture {
        name: "crossed-gradients",
        width: 16,
        height: 16,
        pixel: |x, y| {
            let c = (x * 255 / 15) as u8;
            let a = (y * 255 / 15) as u8;
            [c, c, c, a]
        },
    },
    // A hard alpha edge inside a block, which is what a sprite's outline is and
    // the reason dual plane is worth its weight bits.
    Fixture {
        name: "sprite-edge",
        width: 16,
        height: 16,
        pixel: |x, y| {
            let inside = (4..12).contains(&x) && (4..12).contains(&y);
            if inside {
                [220, 40, 40, 255]
            } else {
                [220, 40, 40, 0]
            }
        },
    },
    // Saturated extremes at the ends of a block's own colour line: 0 and 255
    // must survive exactly, because a texture whose opaque pixels come back at
    // 254 alpha composites visibly wrong and a black that comes back at 4 is a
    // visible lift on a dark scene.
    //
    // The first version of this fixture alternated all four channels
    // independently per texel, which no 4x4 single-partition block can hold:
    // one partition is one colour line, and three independently alternating
    // channels are three. It measured the format's limit rather than the
    // encoder's correctness, and every encoder including a perfect one fails it.
    Fixture {
        name: "extremes",
        width: 24,
        height: 24,
        pixel: |x, _| {
            if x % 4 < 2 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 0]
            }
        },
    },
];

#[test]
#[ignore = "writes fixtures for scripts/test-astc-encoder.sh, which owns the decoder"]
fn emit_astc_fixtures() {
    let directory = PathBuf::from(
        std::env::var("MIGO_ASTC_FIXTURE_DIR")
            .expect("MIGO_ASTC_FIXTURE_DIR must name a directory to write into"),
    );
    std::fs::create_dir_all(&directory).expect("create the fixture directory");
    let mut written = 0usize;

    for fixture in FIXTURES {
        let mut rgba = Vec::with_capacity((fixture.width * fixture.height * 4) as usize);
        for y in 0..fixture.height {
            for x in 0..fixture.width {
                rgba.extend_from_slice(&(fixture.pixel)(x, y));
            }
        }
        // What the ingest path would actually choose for this image, so the
        // gate can hold that one -- and only that one -- to the quality budget.
        // The others are still emitted, because the prediction check applies to
        // all of them and a fault in the infill shows up only in the larger
        // footprints.
        let (_, chosen) = encode_astc_within(&rgba, fixture.width, fixture.height, BUDGET)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));

        // Every footprint, because they share one block layout and differ only
        // in the weight infill -- so a fault in the shared part shows up in all
        // three, and a fault in the infill shows up only in the larger two.
        // Which of the three is worth shipping is a quality question, and the
        // numbers this prints are its input.
        for footprint in [Footprint::X4, Footprint::X6, Footprint::X8] {
            let side = footprint.texels();
            let Some(expected) = footprint.encoded_len(fixture.width, fixture.height) else {
                continue;
            };
            let blocks = encode_astc(&rgba, fixture.width, fixture.height, footprint)
                .unwrap_or_else(|error| panic!("{} at {side}x{side}: {error}", fixture.name));
            assert_eq!(
                blocks.len(),
                expected,
                "{}: one sixteen-byte block per {side}x{side} tile",
                fixture.name
            );
            let stem = format!("{}-{side}", fixture.name);
            std::fs::write(directory.join(format!("{stem}.astc")), &blocks)
                .expect("write the blocks");
            std::fs::write(directory.join(format!("{stem}.rgba")), &rgba)
                .expect("write the source pixels");
            // The encoder's own grade of what it just produced, written out so
            // the oracle can be compared against it. A predictor nobody checks
            // is what picks the wrong footprint quietly: the selection in
            // `encode_astc_within` is only as good as this number, and the only
            // thing that can judge it is a decoder that is not ours.
            let predicted =
                worst_error(&rgba, fixture.width, fixture.height, footprint).expect("aligned");
            std::fs::write(
                directory.join(format!("{stem}.size")),
                format!(
                    "{} {} {side} {predicted} {}\n",
                    fixture.width,
                    fixture.height,
                    u8::from(footprint == chosen)
                ),
            )
            .expect("write the dimensions");
            written += 1;
        }
    }
    println!("wrote {written} ASTC fixtures");
}
