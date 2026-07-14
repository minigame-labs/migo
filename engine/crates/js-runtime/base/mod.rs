use std::{path::PathBuf, sync::Arc};

use deno_core::{Extension, OpState, op2, serde_json, v8};
use shared::op_state::HostOpState;
use tracing::debug;

use crate::io_state::IoSchedulerState;

struct InstallSubpackageRequest {
    zip_path: String,
    pkg_key: String,
    root: String,
    version: String,
    ensure_persistent: bool,
}

fn install_subpackage_blocking(
    mount_table: Arc<shared::vfs::MountTable>,
    game_cache_dir: PathBuf,
    request: InstallSubpackageRequest,
) -> Result<String, String> {
    use shared::vfs::mount::{PackageManifest, StagingArea, package_store_dir};

    let store = package_store_dir(&game_cache_dir);

    let staging = StagingArea::create(&game_cache_dir, &request.pkg_key)
        .map_err(|e| format!("staging create failed: {e}"))?;

    let pkg_filename = format!("{}.mpkg", request.pkg_key);
    let staged_pkg_path = staging.dir().join(&pkg_filename);

    io::ingest_zip_to_package(
        PathBuf::from(&request.zip_path).as_path(),
        &staged_pkg_path,
        &request.pkg_key,
        &request.version,
    )
    .map_err(|e| format!("ingest failed: {e}"))?;

    let final_pkg_path = store.join(&pkg_filename);
    let identity = staging
        .install_package(
            &mount_table,
            &pkg_filename,
            &final_pkg_path,
            &request.root,
            &request.pkg_key,
            &request.version,
        )
        .map_err(|e| format!("install failed: {e}"))?;

    let mut manifest = PackageManifest::load(&store);
    manifest.record(request.pkg_key, request.root, identity.version.clone());
    if let Err(e) = manifest.save(&store) {
        if request.ensure_persistent {
            return Err(format!(
                "manifest write failed (not durably installed): {e}"
            ));
        }
        tracing::warn!("subpackage manifest write failed (package still live): {e}");
    }

    Ok(serde_json::json!({
        "name": identity.name,
        "version": identity.version,
        "checksum": identity.checksum,
    })
    .to_string())
}

async fn install_subpackage_with_scheduler(
    scheduler: Arc<io::scheduler::IoScheduler>,
    mount_table: Arc<shared::vfs::MountTable>,
    game_cache_dir: PathBuf,
    request: InstallSubpackageRequest,
) -> Result<String, String> {
    let compressed_bytes = std::fs::metadata(&request.zip_path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    scheduler
        .run_async(
            io::task::IoRequest::PackageIngest {
                priority: io::task::PriorityClass::Background,
                compressed_bytes,
            },
            move || install_subpackage_blocking(mount_table, game_cache_dir, request),
        )
        .await
        .map_err(|_| "subpackage install worker pool closed".to_string())?
}

/// Derive a collision-free filesystem-safe key from a package name.
/// Uses percent-encoding: every byte that isn't [a-zA-Z0-9._-] is
/// encoded as %XX.  Validates length, traversal, and control chars.
fn safe_package_key(name: &str) -> Result<String, String> {
    let trimmed = name.trim_matches('/');
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(format!("invalid name: empty or too long ({})", name.len()));
    }
    if trimmed.contains("..") || trimmed.bytes().any(|b| b < 0x20) {
        return Err(format!("invalid characters in name: {name}"));
    }
    let mut key = String::with_capacity(trimmed.len());
    for b in trimmed.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                key.push(b as char);
            }
            _ => {
                key.push('%');
                key.push_str(&format!("{:02X}", b));
            }
        }
    }
    Ok(key)
}

#[derive(Debug, thiserror::Error, deno_error::JsError)]
enum RequireError {
    #[class(generic)]
    #[error("{0}")]
    Io(String),
}

/// Trigger a full V8 garbage collection cycle.
///
/// Calls `v8::Isolate::low_memory_notification()` which performs a full GC
/// including both young and old generation collections.
///
/// **Important**: This is a synchronous, stop-the-world operation. It should
/// NOT be called every frame. Appropriate usage:
/// - Scene transitions / level loads
/// - After releasing large resources (textures, audio buffers)
/// - When the game is backgrounded
#[op2(fast)]
fn op_trigger_gc(scope: &mut v8::PinScope<'_, '_>) {
    debug!("triggerGC: requesting V8 full GC via low_memory_notification");
    scope.low_memory_notification();
}

/// Return V8 heap statistics as a JS object.
///
/// Returns `{ totalHeapSize, usedHeapSize, heapSizeLimit, totalPhysicalSize,
///            mallocedMemory, externalMemory }` (all in bytes).
///
#[op2]
#[serde]
fn op_get_heap_statistics(scope: &mut v8::PinScope<'_, '_>) -> HeapStats {
    let stats = scope.get_heap_statistics();
    HeapStats {
        total_heap_size: stats.total_heap_size(),
        used_heap_size: stats.used_heap_size(),
        heap_size_limit: stats.heap_size_limit(),
        total_physical_size: stats.total_physical_size(),
        malloced_memory: stats.malloced_memory(),
        external_memory: stats.external_memory(),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HeapStats {
    total_heap_size: usize,
    used_heap_size: usize,
    heap_size_limit: usize,
    total_physical_size: usize,
    malloced_memory: usize,
    external_memory: usize,
}

/// Check whether `specifier` looks like an absolute filesystem path.
///
/// Matches Unix absolute paths (`/foo`) and Windows drive-letter paths (`C:\foo`).
#[inline]
fn is_absolute_path(specifier: &str) -> bool {
    specifier.starts_with('/')
        || (specifier.len() >= 3
            && specifier.as_bytes()[0].is_ascii_alphabetic()
            && specifier.as_bytes()[1] == b':'
            && matches!(specifier.as_bytes()[2], b'/' | b'\\'))
}

/// Resolve `path` using the Node.js-style extension/index resolution order:
///
/// 1. exact path
/// 2. path.js
/// 3. path.json
/// 4. path/index.js
///
/// Checks both filesystem AND MountTable (for pack-backed overlays where
/// files don't exist on disk but are accessible via the mount).
fn resolve_module_path(
    path: std::path::PathBuf,
    mount_table: Option<&shared::vfs::MountTable>,
    code_dir: &str,
) -> std::path::PathBuf {
    let code_path = std::path::Path::new(code_dir);

    // Helper: check if a candidate path exists on filesystem OR in mount table.
    let exists = |p: &std::path::Path| -> bool {
        if p.is_file() {
            return true;
        }
        // Check mount table for pack-backed entries.
        if let Some(mt) = mount_table {
            if let Ok(rel) = p.strip_prefix(code_path) {
                if let Some(rel_str) = rel.to_str() {
                    return mt.is_file(rel_str);
                }
            }
        }
        false
    };

    if exists(&path) {
        return path;
    }

    // Try appending .js
    if !path.extension().map_or(false, |e| e == "js" || e == "json") {
        let with_js = path.with_extension("js");
        if exists(&with_js) {
            return with_js;
        }
        let with_json = path.with_extension("json");
        if exists(&with_json) {
            return with_json;
        }
    }

    // Try path/index.js (directory as module)
    let index_js = path.join("index.js");
    if exists(&index_js) {
        return index_js;
    }

    // Fall back to original (will produce a clear "not found" error)
    path
}

/// Synchronously read a file as UTF-8 text, used by the JS `require()` shim.
///
/// Resolves `specifier` relative to `referrer_dir`. If `referrer_dir` is empty,
/// falls back to `HostOpState::code_dir`.
#[op2]
#[serde]
fn op_require_resolve_and_read(
    state: &mut OpState,
    #[string] specifier: String,
    #[string] referrer_dir: String,
) -> Result<RequireResult, RequireError> {
    let host = state.borrow::<HostOpState>();
    let mount_table = host.mount_table.clone();
    let code_dir = host.code_dir.clone().unwrap_or_default();
    let _ = host;

    let base_dir = if referrer_dir.is_empty() {
        code_dir.clone()
    } else {
        referrer_dir
    };

    // Reject absolute paths — they must go through /code resolution.
    if is_absolute_path(&specifier) {
        return Err(RequireError::Io(format!(
            "require: absolute path not allowed: {specifier}"
        )));
    }

    let raw_path = std::path::PathBuf::from(&base_dir).join(&specifier);
    let resolved = resolve_module_path(raw_path, mount_table.as_deref(), &code_dir);

    // Compute a normalized relative path for the canonical module key.
    // This ensures ./foo and ./a/../foo produce the same cache key.
    let code_path = std::path::Path::new(&code_dir);
    let normalized_relative = resolved
        .strip_prefix(code_path)
        .ok()
        .and_then(|r| r.to_str())
        .map(|s| {
            // Normalize .. and . textually.
            let mut parts: Vec<&str> = Vec::new();
            for c in s.split('/') {
                match c {
                    "" | "." => {}
                    ".." => {
                        parts.pop();
                    }
                    c => parts.push(c),
                }
            }
            parts.join("/")
        });

    if let (Some(mt), Some(rel)) = (&mount_table, &normalized_relative) {
        // Use resolve() as the single source of truth for overlay shadow semantics.
        // resolve() returns:
        //   Some(real_path=Some) → file on disk (dir-backed overlay or base)
        //   Some(real_path=None) → file in pack-backed overlay
        //   None + overlay matches → shadow: file missing in overlay, don't fall to base
        //   None + no overlay → path not in any overlay, may fall to base filesystem
        let resolved_info = mt.resolve(rel);
        let overlay_claims_subtree = mt.has_overlay_for(rel);

        match &resolved_info {
            Some(info) => {
                // MountTable found the file. Read it.
                match mt.read(rel) {
                    Ok(bytes) => {
                        let content = String::from_utf8(bytes)
                            .map_err(|e| RequireError::Io(format!("require: not UTF-8: {e}")))?;
                        let is_pack = info.real_path.is_none();
                        let abs_path = if is_pack {
                            // Per-source mounted_at: only changes when THIS source
                            // is replaced, not when other overlays change.
                            format!(
                                "{}#s{}",
                                code_path.join(rel).display(),
                                info.source_mounted_at
                            )
                        } else {
                            code_path.join(rel).to_string_lossy().into_owned()
                        };
                        let parent = code_path
                            .join(rel)
                            .parent()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        return Ok(RequireResult {
                            code: content,
                            abs_path,
                            dir: parent,
                        });
                    }
                    Err(e) => {
                        return Err(RequireError::Io(format!(
                            "require: resolved but read failed: {rel}: {e}"
                        )));
                    }
                }
            }
            None if overlay_claims_subtree => {
                // An overlay covers this subtree but the file doesn't exist in it.
                // Shadow: do NOT fall through to base.
                return Err(RequireError::Io(format!(
                    "require: module not found (shadowed by overlay): {rel}"
                )));
            }
            None => {
                // No overlay covers this path. Fall through to base filesystem.
            }
        }
    }

    // Fallback: base filesystem read (only for paths NOT shadowed by an overlay).
    let path = std::fs::canonicalize(&resolved).unwrap_or(resolved);

    // Sandbox: reject paths outside code_dir.
    if !code_dir.is_empty() {
        if !path.starts_with(code_path) {
            return Err(RequireError::Io(format!(
                "require: path escapes /code sandbox: {}",
                path.display()
            )));
        }
    }

    let abs_path = path.to_string_lossy().into_owned();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| RequireError::Io(format!("require: cannot read {}: {}", abs_path, e)))?;
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(RequireResult {
        code: content,
        abs_path,
        dir: parent,
    })
}

#[derive(serde::Serialize)]
struct RequireResult {
    code: String,
    abs_path: String,
    dir: String,
}

/// Returns subpackage definitions as a JSON array, e.g. `[["name","root"],...]`.
/// Returns "[]" if no subpackages are configured.
#[op2]
#[string]
fn op_get_sub_packages(state: &mut OpState) -> String {
    let host = state.borrow::<HostOpState>();
    if host.sub_packages.is_empty() {
        return "[]".to_string();
    }
    let arr: Vec<serde_json::Value> = host
        .sub_packages
        .iter()
        .map(|(name, root)| serde_json::json!({"name": name, "root": root}))
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

/// Returns the workers directory path, or empty string if not configured.
#[op2]
#[string]
fn op_get_workers_path(state: &mut OpState) -> String {
    let host = state.borrow::<HostOpState>();
    host.workers_path.clone().unwrap_or_default()
}

/// Trigger a subpackage download via the platform service.
///
/// The JS layer resolves the subpackage name to a root path from RuntimeConfig,
/// then calls this op to initiate the download. The platform reports progress
/// and completion asynchronously via EvalScript callbacks.
#[op2(fast)]
fn op_download_subpackage(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), deno_error::JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.subpackage() {
            return svc
                .download_subpackage(options_json)
                .map_err(deno_error::JsErrorBox::generic);
        }
    }
    Err(deno_error::JsErrorBox::generic(
        "loadSubpackage:fail not supported",
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;

    use io::{
        scheduler::IoScheduler,
        task::{IoRequest, PriorityClass},
    };
    use shared::vfs::mount::MountTable;

    use super::{
        InstallSubpackageRequest, install_subpackage_blocking, install_subpackage_with_scheduler,
    };

    fn make_test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("migo_base_subpkg_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_test_zip(dir: &std::path::Path) -> std::path::PathBuf {
        let zip_path = dir.join("input.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("main.js", options).unwrap();
        zip.write_all(b"console.log('subpackage')").unwrap();
        zip.finish().unwrap();
        zip_path
    }

    #[test]
    fn subpackage_install_uses_scheduler_ingest_path() {
        let dir = make_test_dir("scheduler_ingest");
        let code_dir = dir.join("code");
        let cache_dir = dir.join("cache");
        std::fs::create_dir_all(&code_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();

        let zip_path = create_test_zip(&dir);
        let mount_table = Arc::new(MountTable::new(code_dir));
        let scheduler = Arc::new(IoScheduler::new(37));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime
            .block_on(install_subpackage_with_scheduler(
                Arc::clone(&scheduler),
                Arc::clone(&mount_table),
                cache_dir.clone(),
                InstallSubpackageRequest {
                    zip_path: zip_path.to_string_lossy().into_owned(),
                    pkg_key: "stage1".to_string(),
                    root: "subpackages/stage1".to_string(),
                    version: "1.0".to_string(),
                    ensure_persistent: false,
                },
            ))
            .unwrap();

        assert_eq!(
            mount_table.read("subpackages/stage1/main.js").unwrap(),
            b"console.log('subpackage')"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subpackage_install_finalize_work_stays_on_archive_worker() {
        let dir = make_test_dir("scheduler_finalize");
        let code_dir = dir.join("code");
        let cache_dir = dir.join("cache");
        std::fs::create_dir_all(&code_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();

        let zip_path = create_test_zip(&dir);
        let mount_table = Arc::new(MountTable::new(code_dir));
        let scheduler = Arc::new(IoScheduler::new(47));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let worker_name = runtime
            .block_on(scheduler.run_async(
                IoRequest::PackageIngest {
                    priority: PriorityClass::Background,
                    compressed_bytes: 0,
                },
                {
                    let mount_table = Arc::clone(&mount_table);
                    let cache_dir = cache_dir.clone();
                    let request = InstallSubpackageRequest {
                        zip_path: zip_path.to_string_lossy().into_owned(),
                        pkg_key: "stage2".to_string(),
                        root: "subpackages/stage2".to_string(),
                        version: "1.0".to_string(),
                        ensure_persistent: false,
                    };
                    move || {
                        install_subpackage_blocking(mount_table, cache_dir, request).unwrap();
                        std::thread::current()
                            .name()
                            .unwrap_or("unnamed")
                            .to_string()
                    }
                },
            ))
            .unwrap();

        assert!(worker_name.starts_with("Migo-IO-"));
        assert_eq!(
            mount_table.read("subpackages/stage2/main.js").unwrap(),
            b"console.log('subpackage')"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Install a subpackage from a downloaded zip file.
///
/// This is the new package-native install path:
/// 1. Ingest zip → .mpkg (in staging area under cache dir)
/// 2. Validate the package (full checksum verification)
/// 3. Atomic rename to final package location
/// 4. Mount as overlay in the MountTable
///
/// Called by JS after a successful subpackage download provides the zip path.
/// Returns a JSON string with the package identity on success.
#[op2(async(lazy), fast)]
#[string]
async fn op_install_subpackage(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] options_json: String,
) -> Result<String, deno_error::JsErrorBox> {
    #[derive(serde::Deserialize)]
    struct InstallOptions {
        #[serde(rename = "zipPath")]
        zip_path: String,
        name: String,
        root: String,
        #[serde(default)]
        version: String,
        /// When true (preDownloadSubpackage), manifest write failure is a hard
        /// error — the caller expects durable installation.  When false
        /// (loadSubpackage), manifest failure is a warning since the package
        /// is live for the current session.
        #[serde(default)]
        ensure_persistent: bool,
    }

    let opts: InstallOptions = serde_json::from_str(&options_json)
        .map_err(|e| deno_error::JsErrorBox::generic(format!("invalid install options: {e}")))?;

    let pkg_key = safe_package_key(&opts.name)
        .map_err(|e| deno_error::JsErrorBox::generic(format!("installSubpackage:fail {e}")))?;

    // Validate root: must be a valid relative path prefix for mount overlay.
    // Reject empty, absolute, traversal, control chars.
    {
        let root = opts.root.trim_matches('/');
        if root.is_empty() {
            return Err(deno_error::JsErrorBox::generic(
                "installSubpackage:fail root is empty",
            ));
        }
        if root.contains("..") || root.contains('\\') || root.bytes().any(|b| b < 0x20) {
            return Err(deno_error::JsErrorBox::generic(format!(
                "installSubpackage:fail invalid root: {}",
                opts.root
            )));
        }
    }

    let (scheduler, mount_table, game_cache_dir) = {
        let st = state.borrow();
        let host = st.borrow::<HostOpState>();

        // Code-signing gate: downloaded subpackages have no Ed25519 signature.
        // When code signing is enforced, reject dynamic installs.  Code-tree
        // subpackages are covered by the base package signing.
        if host.code_signing_enabled {
            return Err(deno_error::JsErrorBox::generic(
                "installSubpackage:fail code signing is enabled; \
                 dynamic subpackage download is not allowed",
            ));
        }

        let mt = host.mount_table.clone().ok_or_else(|| {
            deno_error::JsErrorBox::generic("installSubpackage:fail mount table not initialized")
        })?;
        let gcd = host
            .game_paths
            .as_ref()
            .map(|gp| gp.cache_dir().to_path_buf())
            .ok_or_else(|| {
                deno_error::JsErrorBox::generic("installSubpackage:fail game paths not initialized")
            })?;
        (st.borrow::<IoSchedulerState>().0.clone(), mt, gcd)
    };

    let version = if opts.version.is_empty() {
        "1.0".to_string()
    } else {
        opts.version
    };

    let result = install_subpackage_with_scheduler(
        scheduler,
        mount_table,
        game_cache_dir,
        InstallSubpackageRequest {
            zip_path: opts.zip_path,
            pkg_key,
            root: opts.root,
            version,
            ensure_persistent: opts.ensure_persistent,
        },
    )
    .await
    .map_err(deno_error::JsErrorBox::generic)?;

    Ok(result)
}

/// Get the current MountTable generation counter.
#[op2(fast)]
#[bigint]
fn op_get_mount_generation(state: &mut OpState) -> u64 {
    let host = state.borrow::<HostOpState>();
    host.mount_table
        .as_ref()
        .map(|mt| mt.generation())
        .unwrap_or(0)
}

/// Get the identity of the overlay covering a subpackage root.
/// Returns a stable per-subpackage token (e.g. "subpackage:stage1") that
/// changes only when that specific subpackage is replaced.  Returns empty
/// string if the path is served by the base code tree.
#[op2]
#[string]
fn op_get_subpackage_identity(state: &mut OpState, #[string] root: &str) -> String {
    let host = state.borrow::<HostOpState>();
    match &host.mount_table {
        Some(mt) => mt.overlay_identity_for(root),
        None => String::new(),
    }
}

/// Check if a subpackage is durably installed in the per-game package store.
///
/// Checks manifest.json for an entry where BOTH the package key matches
/// the name AND the prefix matches the root, AND the .mpkg file exists.
/// This prevents false positives from stale or mismatched manifest entries.
#[op2(fast)]
fn op_is_subpackage_persisted(
    state: &mut OpState,
    #[string] name: &str,
    #[string] root: &str,
) -> bool {
    let host = state.borrow::<HostOpState>();
    let Some(game_paths) = &host.game_paths else {
        return false;
    };
    let store = shared::vfs::mount::package_store_dir(game_paths.cache_dir());
    let manifest = shared::vfs::mount::PackageManifest::load(&store);

    // Derive the same package key that install would use.
    let pkg_key = match safe_package_key(name) {
        Ok(k) => k,
        Err(_) => return false,
    };

    // Look up by derived key.
    if let Some(entry) = manifest.packages.get(&pkg_key) {
        if entry.prefix == root {
            let pkg_path = store.join(format!("{pkg_key}.mpkg"));
            return pkg_path.exists();
        }
    }
    false
}

/// Check if a subpackage is already available locally.
///
/// Returns true if the subpackage content is accessible via MountTable
/// (either as an installed .mpkg overlay or as files in the base code tree).
/// Used by JS to skip download when the subpackage is already present.
#[op2(fast)]
fn op_is_subpackage_installed(state: &mut OpState, #[string] root: &str) -> bool {
    let host = state.borrow::<HostOpState>();
    let Some(mt) = &host.mount_table else {
        return false;
    };

    // Check if any entry point candidate exists in the mount view.
    let candidates = [
        format!("{root}/game.js"),
        format!("{root}/index.js"),
        format!("{root}/main.js"),
    ];
    for candidate in &candidates {
        if mt.exists(candidate) || mt.exists_or_is_dir(candidate) {
            return true;
        }
    }
    // Also check if the root itself is a visible directory with content.
    !mt.list_dir(root).is_empty()
}

deno_core::extension!(
    host_v8_base,
    ops = [
        op_trigger_gc,
        op_get_heap_statistics,
        op_require_resolve_and_read,
        op_get_sub_packages,
        op_get_workers_path,
        op_download_subpackage,
        op_install_subpackage,
        op_get_mount_generation,
        op_get_subpackage_identity,
        op_is_subpackage_persisted,
        op_is_subpackage_installed,
    ],
    esm = [
        dir "base",
        "01_amdshim.js",
        "02_async.js",
        "03_gc.js",
        "04_subpackage.js",
        "05_perf.js",
    ],
    options = {
        options: HostOpState,
    },
    state = |state, options| {
        state.put::<HostOpState>(options.options);
    },
);

pub fn base_extensions(host: HostOpState) -> Vec<Extension> {
    vec![host_v8_base::init(host)]
}
