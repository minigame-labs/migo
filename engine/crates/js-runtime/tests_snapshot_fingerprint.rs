//! Regression tests for the V8-snapshot JS fingerprint (Q1 review-2 P2-A).
//!
//! The fingerprint is computed independently in two places that MUST agree:
//!   * `build.rs` (compile-time: decides whether to embed a snapshot), through
//!     the shared `build_snapshot.rs` helper `include!`d below, and
//!   * `scripts/lib/snapshot-fingerprint.sh` (snapshot generation + freshness).
//! If they disagree, a freshly generated snapshot is silently rejected at build
//! time (fail-safe, but the cold-start win is lost). These tests pin the Rust
//! helper to filesystem enumeration + `LC_ALL=C` relative-path byte ordering and
//! prove it needs no `.git` directory.

#[allow(dead_code)]
mod build_snapshot {
    include!("build_snapshot.rs");
}

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use sha2::{Digest, Sha256};

use build_snapshot::{collect_js_files, snapshot_js_hash};

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn rel(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .expect("path under repo root")
        .to_string_lossy()
        .replace('\\', "/")
}

/// Independent reference for the shell pipeline
/// `find ... | LC_ALL=C sort | xargs sha256sum | sha256sum`: order files by the
/// raw bytes of their repo-root-relative path, emit `"<filehash>  <relpath>\n"`,
/// hash the concatenation.
fn reference_byte_order(repo_root: &Path, files: &[PathBuf]) -> String {
    let mut entries: Vec<(String, PathBuf)> = files
        .iter()
        .map(|p| (rel(repo_root, p), p.clone()))
        .collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut outer = Sha256::new();
    for (relpath, path) in &entries {
        let file_hash = hex(&Sha256::digest(std::fs::read(path).unwrap()));
        outer.update(format!("{file_hash}  {relpath}\n").as_bytes());
    }
    hex(&outer.finalize())
}

/// The pre-fix behavior: order by `Path`'s component-wise `Ord` (what
/// `Vec<PathBuf>::sort()` did). Kept only to prove the fixture actually triggers
/// the divergence the fix addresses.
fn reference_component_order(repo_root: &Path, files: &[PathBuf]) -> String {
    let mut entries: Vec<PathBuf> = files.to_vec();
    entries.sort();
    let mut outer = Sha256::new();
    for path in &entries {
        let relpath = rel(repo_root, path);
        let file_hash = hex(&Sha256::digest(std::fs::read(path).unwrap()));
        outer.update(format!("{file_hash}  {relpath}\n").as_bytes());
    }
    hex(&outer.finalize())
}

fn unique_tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "migo-snap-fp-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn fingerprint_uses_lc_all_c_byte_order_not_path_component_order() {
    let root = unique_tmp_dir("order");
    let jsr = root.join("engine/crates/js-runtime");

    // `worker.js` (top level) vs `worker/x.js` (nested): byte-wise `.` (0x2e) <
    // `/` (0x2f) puts `worker.js` first, but `Path` component order puts
    // `worker/x.js` first. `app-init.js` vs `app/init.js` is a second `-`
    // (0x2d) < `/` divergence.
    write_file(&jsr.join("worker.js"), b"top-level worker\n");
    write_file(&jsr.join("worker/x.js"), b"nested worker\n");
    write_file(&jsr.join("app-init.js"), b"a\n");
    write_file(&jsr.join("app/init.js"), b"b\n");
    write_file(&jsr.join("web/02_timers.js"), b"timers\n");
    write_file(&jsr.join("notes.txt"), b"ignored\n");

    let files = collect_js_files(&jsr).unwrap();

    let byte_order = reference_byte_order(&root, &files);
    let component_order = reference_component_order(&root, &files);
    assert_ne!(
        byte_order, component_order,
        "the fixture must actually exercise the component-vs-byte divergence"
    );

    let actual = snapshot_js_hash(&root, &files).unwrap();
    assert_eq!(
        actual, byte_order,
        "snapshot_js_hash must order by LC_ALL=C relative-path bytes, matching \
         scripts/lib/snapshot-fingerprint.sh"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fingerprint_enumerates_every_js_on_disk_and_ignores_non_js() {
    let root = unique_tmp_dir("set");
    let jsr = root.join("engine/crates/js-runtime");
    write_file(&jsr.join("98_glob.js"), b"g\n");
    write_file(&jsr.join("web/03_canvas.js"), b"c\n");
    write_file(&jsr.join("worker/02_worker_inner.js"), b"w\n");
    write_file(&jsr.join("README.md"), b"doc\n");
    write_file(&jsr.join("web/notes.txt"), b"nope\n");
    write_file(&jsr.join("web/upper.JS"), b"case\n"); // extension match is case-sensitive

    let files = collect_js_files(&jsr).unwrap();
    let mut found: Vec<String> = files.iter().map(|p| rel(&root, p)).collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            "engine/crates/js-runtime/98_glob.js".to_string(),
            "engine/crates/js-runtime/web/03_canvas.js".to_string(),
            "engine/crates/js-runtime/worker/02_worker_inner.js".to_string(),
        ]
    );

    assert_eq!(
        snapshot_js_hash(&root, &files).unwrap(),
        reference_byte_order(&root, &files)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fingerprint_works_without_a_git_directory() {
    let root = unique_tmp_dir("nogit");
    let jsr = root.join("engine/crates/js-runtime");
    write_file(&jsr.join("web/a.js"), b"x\n");
    assert!(!root.join(".git").exists(), "fixture must have no .git");

    let files = collect_js_files(&jsr).unwrap();
    let h = snapshot_js_hash(&root, &files).unwrap();
    assert_eq!(h.len(), 64);
    assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
    let _ = std::fs::remove_dir_all(&root);
}

/// Cross-language equivalence on the real tree: the shared shell helper (sourced
/// exactly as gen-snapshot / check-snapshot-freshness use it) and the Rust
/// `snapshot_js_hash` must produce the same digest over the live
/// `engine/crates/js-runtime` extension JS. This is what guarantees a freshly
/// generated snapshot's manifest hash matches `build.rs` at embed time.
/// Linux-only: the shell helper relies on GNU `sha256sum`/`xargs` on the build
/// host; the three tests above cover the pure-Rust behavior everywhere.
#[cfg(target_os = "linux")]
#[test]
fn rust_and_shell_fingerprints_match_on_the_real_tree() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(3)
        .expect("js-runtime lives under <repo>/engine/crates");

    let files = collect_js_files(manifest_dir).unwrap();
    assert!(!files.is_empty(), "expected extension JS under js-runtime");
    let rust_hash = snapshot_js_hash(repo_root, &files).unwrap();

    let script = repo_root.join("scripts/lib/snapshot-fingerprint.sh");
    let output = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!(
            "source '{}' && snapshot_js_hash '{}'",
            script.display(),
            repo_root.display()
        ))
        .output()
        .expect("run scripts/lib/snapshot-fingerprint.sh");
    assert!(
        output.status.success(),
        "shell fingerprint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shell_hash = String::from_utf8(output.stdout).unwrap().trim().to_string();

    assert_eq!(
        rust_hash, shell_hash,
        "build.rs (Rust) and scripts/lib/snapshot-fingerprint.sh must agree \
         byte-for-byte on the real extension-JS tree"
    );
}

/// Enumeration is atomic from the caller's perspective: pointing at a
/// non-directory fails outright and returns no `Vec`. The `io::Result<Vec>`
/// signature is what makes it impossible for `build.rs` to observe a partial
/// set and hash it into a false match against a stale manifest.
#[test]
fn enumeration_of_a_non_directory_is_err() {
    let root = unique_tmp_dir("enum-nondir");
    let not_a_dir = root.join("engine/crates/js-runtime");
    write_file(&not_a_dir, b"i am a file, not a directory\n");

    assert!(
        collect_js_files(&not_a_dir).is_err(),
        "enumerating a non-directory must return Err, not a partial/empty Ok"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// The correctness red line: a subdirectory the walk cannot read must fail the
/// WHOLE enumeration, never silently drop it and return the partial set already
/// collected (which could hash-match a stale manifest and accept an outdated
/// snapshot). Guards against a future "lenient" walk that swallows read errors.
#[cfg(unix)]
#[test]
fn unreadable_subdir_fails_enumeration_without_a_partial_set() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_tmp_dir("enum-locked");
    let jsr = root.join("engine/crates/js-runtime");
    // A collectible top-level file plus an unreadable subdir that also contains
    // JS, so a lenient walk would be tempted to return just the top-level set.
    write_file(&jsr.join("keep.js"), b"keep\n");
    let locked = jsr.join("locked");
    write_file(&locked.join("inner.js"), b"inner\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

    if std::fs::read_dir(&locked).is_ok() {
        // Privileged runs (e.g. root in CI) bypass the 0o000 bit, so the
        // permission-based failure cannot be simulated here — restore and skip.
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    assert!(
        collect_js_files(&jsr).is_err(),
        "an unreadable subdirectory must fail enumeration, not yield a partial set"
    );

    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
    let _ = std::fs::remove_dir_all(&root);
}

/// The manifest-writing path must not hard-depend on git for locating the repo
/// root (`git rev-parse` exits 128 without an accessible `.git`). Run the real
/// script with a `git` that always fails and assert it still writes a valid
/// manifest (ROOT derived from the script's own location, `git_commit`
/// best-effort `unknown`).
#[cfg(target_os = "linux")]
#[test]
fn write_snapshot_manifest_succeeds_without_git() {
    use std::os::unix::fs::PermissionsExt;

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(3)
        .expect("js-runtime lives under <repo>/engine/crates");
    let script = repo_root.join("scripts/write-snapshot-manifest.sh");

    let tmp = unique_tmp_dir("manifest-nogit");
    // A fake `git` on PATH that always fails, so this is a real no-git
    // regression regardless of the host's actual .git accessibility.
    let fake_bin = tmp.join("bin");
    let fake_git = fake_bin.join("git");
    write_file(&fake_git, b"#!/bin/sh\nexit 128\n");
    std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).unwrap();

    let bin = tmp.join("SNAPSHOT-testarch.bin");
    std::fs::write(&bin, b"dummy snapshot bytes").unwrap();
    let manifest = PathBuf::from(format!("{}.manifest.json", bin.display()));

    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = std::process::Command::new("bash")
        .arg(&script)
        .arg("testarch")
        .arg(&bin)
        .env("PATH", path_env)
        .output()
        .expect("run scripts/write-snapshot-manifest.sh");
    assert!(
        output.status.success(),
        "write-snapshot-manifest.sh must succeed without git (ROOT from SCRIPT_DIR, \
         git_commit best-effort); stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let written = std::fs::read_to_string(&manifest).expect("manifest written");
    assert!(
        written.contains("\"arch\": \"testarch\""),
        "manifest: {written}"
    );
    assert!(
        written.contains("\"js_sources_sha256\""),
        "manifest: {written}"
    );
    assert!(
        written.contains("\"deno_core_version\""),
        "manifest: {written}"
    );
    assert!(
        written.contains("\"git_commit\": \"unknown\""),
        "git_commit must degrade to unknown without git; manifest: {written}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
