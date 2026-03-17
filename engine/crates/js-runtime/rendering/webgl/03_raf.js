import {
    op_set_preferred_fps,
    op_await_next_frame,
} from "ext:core/ops";

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
        // Consecutive frames with zero RAF callbacks before we stop the loop.
        // The loop restarts on the next requestAnimationFrame() call.
        let idleFrames = 0;
        const MAX_IDLE_FRAMES = 3;

        while (true) {
            const ts = await op_await_next_frame();

            const callbacks = __raf_callbacks;
            __raf_callbacks = Object.create(null);
            const ids = Object.keys(callbacks);
            let shouldStop = false;

            if (ids.length === 0) {
                // No callbacks registered -- stop loop after a grace period
                // to avoid restart churn if a callback is re-registered next frame.
                if (++idleFrames >= MAX_IDLE_FRAMES) {
                    shouldStop = true;
                }
            } else {
                idleFrames = 0;
                for (let i = 0; i < ids.length; i++) {
                    try {
                        callbacks[ids[i]](ts);
                    } catch (e) {
                        console.error('RAF callback error:', e);
                    }
                }
            }

            // Flush all batched canvas commands at end of frame.
            if (globalThis.__migo_frame_end_all) {
                globalThis.__migo_frame_end_all();
            }

            if (shouldStop) {
                break;
            }
        }
    } catch (e) {
        console.error('RAF loop terminated:', e);
    }
    __rafLoopRunning = false;
}

const setPreferredFramesPerSecond = (fps) => {
    op_set_preferred_fps(Math.max(1, Math.min(60, fps | 0)));
};

export { requestAnimationFrame, cancelAnimationFrame, setPreferredFramesPerSecond };
