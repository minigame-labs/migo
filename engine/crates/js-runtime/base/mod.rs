use deno_core::{Extension, OpState, op2, v8};
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
        host.code_dir
            .clone()
            .unwrap_or_default()
    } else {
        referrer_dir
    };

    // Resolve the specifier to an absolute path
    let path = if specifier.starts_with('/') || specifier.contains(':') {
        std::path::PathBuf::from(&specifier)
    } else {
        std::path::PathBuf::from(&base_dir).join(&specifier)
    };

    // Try with and without .js extension
    let path = if path.exists() {
        path
    } else if !path.extension().map_or(false, |e| e == "js") {
        let with_js = path.with_extension("js");
        if with_js.exists() {
            with_js
        } else {
            path
        }
    } else {
        path
    };

    let abs_path = path.to_string_lossy().into_owned();

    let content = std::fs::read_to_string(&path).map_err(|e| {
        RequireError::Io(format!("require: cannot read {}: {}", abs_path, e))
    })?;

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

deno_core::extension!(
    host_v8_base,
    ops = [
        op_trigger_gc,
        op_get_heap_statistics,
        op_require_resolve_and_read,
    ],
    esm = [
        dir "base",
        "01_amdshim.js",
        "02_async.js",
        "03_gc.js",
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
