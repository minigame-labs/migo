// Shared V8-snapshot input fingerprint helpers.
//
// `include!`d verbatim by both `build.rs` (the compile-time embed decision) and
// the `tests_snapshot_fingerprint` test module, so the two stay byte-identical.
// (`include!` is why this uses `//` rather than `//!` — inner doc comments are
// rejected when pasted inside a `mod` block.) Kept dependency-light (only `sha2`
// + `std`): `build.rs` pulls `sha2` in as a build-dependency and the test pulls
// it in as a dev-dependency.
//
// The result MUST equal the shell pipeline in
// `scripts/lib/snapshot-fingerprint.sh`:
//
//   find engine/crates/js-runtime -type f -name '*.js' \
//     | LC_ALL=C sort | xargs -r -d '\n' sha256sum | sha256sum
//
// Both sides therefore agree on:
//   * the file set — every `*.js` on disk under `engine/crates/js-runtime`
//     (a filesystem walk, NOT a git query) so the no-`.git` workspace works and
//     untracked/generated embedded JS also invalidates a snapshot; and
//   * the order — the raw bytes of the repo-root-relative, forward-slash path
//     (`LC_ALL=C`), which is NOT the same as `Path`'s component-wise `Ord`
//     (e.g. `worker.js` sorts before `worker/x.js` byte-wise because `.` (0x2e)
//     < `/` (0x2f), but after it component-wise).
//
// Each hashed line is `"<sha256hex>  <relpath>\n"` (two spaces, matching the
// `sha256sum` filename column), fed into an outer SHA-256.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Recursively collect every `*.js` file under `dir` via a filesystem walk (not
/// a git query), so untracked/generated embedded JS also invalidates a snapshot
/// and enumeration works without a `.git` directory. Symlinks are skipped
/// (`file_type()` does not follow them), matching `find` without `-L`.
///
/// Enumeration is atomic from the caller's perspective: on ANY read failure
/// (e.g. an unreadable or newly-added directory the traversal cannot open) the
/// whole call returns `Err` and no partial `Vec` is produced. This is a
/// correctness red line — a truncated set could hash-match a stale manifest and
/// wrongly accept an outdated snapshot, so callers must never validate/embed on
/// error and instead fall back to source JS.
pub fn collect_js_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut output = Vec::new();
    collect_js_files_into(dir, &mut output)?;
    Ok(output)
}

fn collect_js_files_into(dir: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_js_files_into(&path, output)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "js") {
            output.push(path);
        }
    }
    Ok(())
}

/// Repo-root-relative, forward-slash path. Used both for ordering and for the
/// hashed line (matches the `sha256sum` filename column emitted by the shell
/// pipeline). On the Linux build host paths are already `/`-separated; the
/// `\\` → `/` replacement is only a Windows-host safety net.
pub fn relative_slash_path(path: &Path, repo_root: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(repo_root)
        .map_err(|error| format!("JS path {} is outside repo: {error}", path.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

/// Fingerprint of all extension JS. Order-independent of the `js_files` slice:
/// it sorts internally so callers need not pre-sort.
pub fn snapshot_js_hash(repo_root: &Path, js_files: &[PathBuf]) -> Result<String, String> {
    let mut entries: Vec<(String, &Path)> = Vec::with_capacity(js_files.len());
    for path in js_files {
        entries.push((relative_slash_path(path, repo_root)?, path.as_path()));
    }
    // Order by the raw bytes of the repo-root-relative path (LC_ALL=C), matching
    // `sort` in scripts/lib/snapshot-fingerprint.sh. This is deliberately NOT
    // `Path`'s component-wise `Ord` (`Vec::<PathBuf>::sort()`): the two disagree
    // whenever a filename byte is below `/` (0x2f) — e.g. `worker.js` vs
    // `worker/x.js` — which would make the two fingerprints diverge.
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut outer = Sha256::new();
    for (relative, path) in &entries {
        let file_hash = sha256_file(path)
            .map_err(|error| format!("cannot hash JS {}: {error}", path.display()))?;
        outer.update(format!("{file_hash}  {relative}\n").as_bytes());
    }
    Ok(hex_digest(outer.finalize().as_slice()))
}

pub fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let bytes = std::fs::read(path)?;
    Ok(hex_digest(Sha256::digest(bytes).as_slice()))
}

pub fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
