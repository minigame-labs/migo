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

async function _startRafLoop() {
    try {
        let idleFrames = 0;
        const MAX_IDLE_FRAMES = 3;

        while (true) {
            const ts = await op_await_next_frame();

            const callbacks = __raf_callbacks;
            __raf_callbacks = Object.create(null);
            const ids = Object.keys(callbacks);
            let shouldStop = false;

            if (ids.length === 0) {
                if (++idleFrames >= MAX_IDLE_FRAMES) {
                    shouldStop = true;
                }
            } else {
                idleFrames = 0;
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

            // Flush outside profiling block for idle frames
            if (ids.length === 0 && globalThis.__migo_frame_end_all) {
                globalThis.__migo_frame_end_all();
            }

            if (shouldStop) {
                break;
            }
        }
    } catch (e) {
        console.error(`RAF loop terminated: ${errorToString(e)}`);
    }
    __rafLoopRunning = false;
}

// Native lifecycle hook: when Android returns from background, there may
// already be RAF callbacks queued while the async loop has stopped after idle
// frames.  Restarting the loop is cheap (it will exit again after a few idle
// ticks) and mirrors browser engines kicking their compositor/RAF scheduler on
// visibility changes.  Exposed as a global so Host::on_update_surface can call
// it immediately after the render thread has a valid surface again.
globalThis.__migo_restart_raf_loop = function () {
    if (!__rafLoopRunning) {
        __rafLoopRunning = true;
        _startRafLoop();
    }
};

const setPreferredFramesPerSecond = (fps) => {
    op_set_preferred_fps(Math.max(1, Math.min(120, fps | 0)));
};

export { requestAnimationFrame, cancelAnimationFrame, setPreferredFramesPerSecond };
