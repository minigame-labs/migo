import {
    op_set_preferred_fps,
    op_await_next_frame,
} from "ext:core/ops";
import { errorToString } from "ext:host_v8_base/02_async.js";

let __nextRafId = 0;
let __raf_callbacks = Object.create(null);
let __rafLoopRunning = false;

const requestAnimationFrame = (cb) => {
    const id = ++__nextRafId;
    __raf_callbacks[id] = cb;
    if (!__rafLoopRunning) {
        __rafLoopRunning = true;
        _startRafLoop();
    }
    return id;
};

const cancelAnimationFrame = (id) => {
    delete __raf_callbacks[id];
};

// R1 demand-driven RAF loop: awaits a frame signal only while callbacks are
// queued, and stops the instant none remain (no idle-frame tail). Under the
// on-demand VSync model the render thread arms exactly one frame per pending
// waiter, so an idle tail would leave the display clock running with nothing to
// draw.
async function _startRafLoop() {
    try {
        while (true) {
            // Stop immediately when no callbacks are queued. This check and the
            // `__rafLoopRunning = false` below run synchronously with no `await`
            // in between, so a requestAnimationFrame() call (which cannot
            // interrupt synchronous JS) either observed a non-empty queue before
            // this check (we await) or runs after we return (it restarts the
            // loop). Either way the request is never lost.
            if (Object.keys(__raf_callbacks).length === 0) {
                __rafLoopRunning = false;
                return;
            }

            const ts = await op_await_next_frame();

            // Snapshot + clear AFTER the await so callbacks registered during the
            // wait (including while backgrounded) are included, and callbacks
            // that (re-)register from within a callback continue the loop.
            const callbacks = __raf_callbacks;
            __raf_callbacks = Object.create(null);
            const ids = Object.keys(callbacks);

            for (let i = 0; i < ids.length; i++) {
                try {
                    callbacks[ids[i]](ts);
                } catch (e) {
                    console.error('RAF callback error: ' + errorToString(e));
                }
            }

            if (globalThis.__migo_frame_end_all) {
                globalThis.__migo_frame_end_all();
            }
        }
    } catch (e) {
        console.error(`RAF loop terminated: ${errorToString(e)}`);
    }
    __rafLoopRunning = false;
}

// Native lifecycle hook: when Android returns from background there may be RAF
// callbacks that were queued while the loop had stopped. Restart the loop so
// they run -- but only if there is actually something queued, otherwise
// restarting would arm a one-shot VSync with no work to do (R1: no wasted frame
// on resume when idle). Mirrors browser engines kicking their compositor/RAF
// scheduler on visibility changes. Exposed as a global so Host::enter_foreground
// can call it once the render thread has a valid surface again.
Object.defineProperty(globalThis, "__migo_restart_raf_loop", {
    // Non-enumerable for the same reason as the frame-end hooks: the host
    // reaches this by name, content should not meet it while enumerating.
    value: function () {
        if (!__rafLoopRunning && Object.keys(__raf_callbacks).length > 0) {
            __rafLoopRunning = true;
            _startRafLoop();
        }
    },
    enumerable: false,
    writable: true,
    configurable: true,
});

const setPreferredFramesPerSecond = (fps) => {
    op_set_preferred_fps(Math.max(1, Math.min(120, fps | 0)));
};

export { requestAnimationFrame, cancelAnimationFrame, setPreferredFramesPerSecond };
