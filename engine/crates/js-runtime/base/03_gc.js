// GC and memory introspection APIs.
//
// triggerGC()         - triggers a full V8 GC cycle
// getHeapStatistics() - returns V8 heap usage info (totalHeapSize, usedHeapSize, ...)

import { core } from "ext:core/mod.js";

/**
 * Trigger a full V8 garbage collection cycle.
 *
 * Calls V8's `low_memory_notification()` which performs a stop-the-world
 * mark-sweep-compact GC on both young and old generation.
 *
 * Best used during scene transitions, after releasing large resources,
 * or when the game is backgrounded. Do NOT call every frame.
 */
function triggerGC() {
    core.ops.op_trigger_gc();
}

/**
 * Get current V8 heap statistics.
 *
 * @returns {{ totalHeapSize: number, usedHeapSize: number, heapSizeLimit: number,
 *             totalPhysicalSize: number, mallocedMemory: number, externalMemory: number }}
 */
function getHeapStatistics() {
    return core.ops.op_get_heap_statistics();
}

export { triggerGC, getHeapStatistics };
