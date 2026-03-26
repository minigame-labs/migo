use deno_core::{op2, serde_json, v8, Extension, OpState};
use shared::op_state::HostOpState;
use tracing::debug;

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
fn resolve_module_path(path: std::path::PathBuf) -> std::path::PathBuf {
    if path.is_file() {
        return path;
    }

    // Try appending .js
    if !path.extension().map_or(false, |e| e == "js" || e == "json") {
        let with_js = path.with_extension("js");
        if with_js.is_file() {
            return with_js;
        }
        let with_json = path.with_extension("json");
        if with_json.is_file() {
            return with_json;
        }
    }

    // Try path/index.js (directory as module)
    let index_js = path.join("index.js");
    if index_js.is_file() {
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
    let base_dir = if referrer_dir.is_empty() {
        let host = state.borrow::<HostOpState>();
        host.code_dir.clone().unwrap_or_default()
    } else {
        referrer_dir
    };

    // Resolve the specifier to an absolute path
    let raw_path = if is_absolute_path(&specifier) {
        std::path::PathBuf::from(&specifier)
    } else {
        std::path::PathBuf::from(&base_dir).join(&specifier)
    };

    // Apply extension/index resolution
    let resolved = resolve_module_path(raw_path);

    // Canonicalize to normalize symlinks, `..`, and produce a unique cache key.
    let path = std::fs::canonicalize(&resolved).unwrap_or(resolved);

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

deno_core::extension!(
    host_v8_base,
    ops = [
        op_trigger_gc,
        op_get_heap_statistics,
        op_require_resolve_and_read,
        op_get_sub_packages,
        op_get_workers_path,
        op_download_subpackage,
    ],
    esm = [
        dir "base",
        "01_amdshim.js",
        "02_async.js",
        "03_gc.js",
        "04_subpackage.js",
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
