use deno_core::{Extension, OpState, op2, v8};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;
use shared::protocol::host_cmd::HostCommand;
use tracing::debug;

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

#[op2(fast)]
pub fn op_restart_mini_program(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    host.host_tx
        .try_send(HostCommand::Restart)
        .map_err(|e| JsErrorBox::generic(format!("restartMiniProgram:fail {e}")))
}

#[op2(fast)]
pub fn op_exit_mini_program(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    host.host_tx
        .try_send(HostCommand::Shutdown)
        .map_err(|e| JsErrorBox::generic(format!("exitMiniProgram:fail {e}")))
}

deno_core::extension!(
    host_v8_base,
    ops = [
        op_trigger_gc,
        op_get_heap_statistics,
        op_restart_mini_program,
        op_exit_mini_program,
    ],
    esm = [
        dir "base",
        "01_amdshim.js",
        "02_async.js",
        "03_gc.js",
        "04_lifecycle.js",
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
