//! The fixed corpus both encoders are measured against.
//!
//! Not a round-trip test. A round trip proves the reader understands the
//! writer, which is true of any pair of implementations that share a mistake.
//! These are committed bytes: the JavaScript producer running inside WebContent
//! must emit exactly these for the same input, and any reader must accept or
//! reject each one exactly as `index.json` says.
//!
//! A diff here is a wire-format change. It is answered with a version decision,
//! not by regenerating the file.

use std::{fs, path::PathBuf};

use frame_wire::{
    FLAG_CONTINUED, FLAG_PRESENT, SECTION_KIND_COMMAND_STREAM, SECTION_KIND_DAMAGE,
    SECTION_KIND_INLINE_DATA, SECTION_KIND_RESOURCE_REFERENCES, builder::WireFrameBuilder,
    validate,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../contracts/frame-wire/golden")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/frame-wire/golden")
        })
}

struct Case {
    name: &'static str,
    bytes: Vec<u8>,
}

/// Deterministic by construction: no timestamps, no randomness, no allocator
/// addresses. A corpus that changes between runs cannot be committed.
fn cases() -> Vec<Case> {
    let mut cases = Vec::new();

    // The smallest legal packet: one command stream, present, nothing else.
    let empty_stream: [u8; 0] = [];
    let mut minimal = WireFrameBuilder::new();
    minimal.session_nonce = 0;
    minimal.sequence = 1;
    minimal.runtime_generation = 1;
    minimal.frame_id = 1;
    minimal.flags = FLAG_PRESENT;
    cases.push(Case {
        name: "minimal-present",
        bytes: minimal
            .section(SECTION_KIND_COMMAND_STREAM, 0, &empty_stream)
            .build(),
    });

    // A frame shaped like a real one: commands, an inline blob whose length is
    // not a multiple of 8 (so the encoder's padding is pinned), resources, and
    // an advisory damage section.
    let stream: Vec<u8> = (0..64u8).collect();
    let inline: Vec<u8> = (0..21u8).map(|value| value.wrapping_mul(7)).collect();
    let resources: Vec<u8> = (0..12u8).map(|value| value.wrapping_add(200)).collect();
    let damage: Vec<u8> = (0..16u8).map(|value| value.wrapping_mul(3)).collect();
    let mut typical = WireFrameBuilder::new();
    typical.session_nonce = 0x0123_4567_89AB_CDEF;
    typical.sequence = 42;
    typical.runtime_generation = 3;
    typical.surface_generation = 9;
    typical.resource_epoch = 2;
    typical.frame_id = 41;
    typical.flags = FLAG_PRESENT;
    cases.push(Case {
        name: "typical-frame",
        bytes: typical
            .section(SECTION_KIND_COMMAND_STREAM, 16, &stream)
            .section(SECTION_KIND_INLINE_DATA, 21, &inline)
            .section(SECTION_KIND_RESOURCE_REFERENCES, 3, &resources)
            .section(SECTION_KIND_DAMAGE, 1, &damage)
            .build(),
    });

    // A continuation: same frame_id, no present. Pins that CONTINUED is bit 1
    // and that a non-presenting packet is legal.
    let more: Vec<u8> = (0..32u8).rev().collect();
    let mut continued = WireFrameBuilder::new();
    continued.session_nonce = 0x0123_4567_89AB_CDEF;
    continued.sequence = 43;
    continued.runtime_generation = 3;
    continued.surface_generation = 9;
    continued.resource_epoch = 2;
    continued.frame_id = 41;
    continued.flags = FLAG_CONTINUED;
    cases.push(Case {
        name: "continued-no-present",
        bytes: continued
            .section(SECTION_KIND_COMMAND_STREAM, 8, &more)
            .build(),
    });

    cases
}

fn sha256_hex(bytes: &[u8]) -> String {
    // A small, self-contained SHA-256 so the corpus check does not pull a
    // dependency into a crate whose whole point is a small trust boundary.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_length = (bytes.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[test]
fn the_committed_corpus_matches_this_encoder() {
    let dir = corpus_dir();
    let updating = std::env::var_os("MIGO_UPDATE_FRAME_WIRE_GOLDEN").is_some();
    if updating {
        fs::create_dir_all(&dir).expect("create corpus directory");
    }

    let mut index_lines = Vec::new();
    let mut mismatches = Vec::new();

    for case in cases() {
        let path = dir.join(format!("{}.bin", case.name));
        let digest = sha256_hex(&case.bytes);
        index_lines.push(format!(
            "    {{ \"name\": \"{}\", \"bytes\": {}, \"sha256\": \"{}\", \"accepted\": true }}",
            case.name,
            case.bytes.len(),
            digest
        ));

        if updating {
            fs::write(&path, &case.bytes).expect("write corpus case");
            continue;
        }

        match fs::read(&path) {
            Ok(committed) => {
                if committed != case.bytes {
                    mismatches.push(format!(
                        "{}: committed {} bytes, encoder produced {}",
                        case.name,
                        committed.len(),
                        case.bytes.len()
                    ));
                }
                validate(&committed).unwrap_or_else(|error| {
                    panic!("committed case {} no longer validates: {error}", case.name)
                });
            }
            Err(error) => mismatches.push(format!("{}: unreadable ({error})", case.name)),
        }
    }

    if updating {
        let index = format!("{{\n  \"cases\": [\n{}\n  ]\n}}\n", index_lines.join(",\n"));
        fs::write(dir.join("index.json"), index).expect("write corpus index");
        panic!(
            "corpus regenerated. Re-run without MIGO_UPDATE_FRAME_WIRE_GOLDEN, and treat the diff \
             as a wire-format change needing a version decision."
        );
    }

    assert!(
        mismatches.is_empty(),
        "the encoder no longer produces the committed bytes:\n  {}",
        mismatches.join("\n  ")
    );

    let index = fs::read_to_string(dir.join("index.json")).expect("corpus index");
    for case in cases() {
        assert!(
            index.contains(&format!("\"{}\"", case.name)),
            "case {} is missing from index.json",
            case.name,
        );
        assert!(
            index.contains(&sha256_hex(&case.bytes)),
            "case {} has a stale digest in index.json",
            case.name,
        );
    }
}

/// The corpus is worth nothing if it is empty, and an empty directory would
/// otherwise report a clean pass.
#[test]
fn the_corpus_is_not_empty() {
    assert!(
        cases().len() >= 3,
        "the corpus must cover more than one shape"
    );
}
