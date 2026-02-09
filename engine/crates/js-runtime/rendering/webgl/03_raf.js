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
        while (true) {
            const ts = await op_await_next_frame();

            const callbacks = __raf_callbacks;
            __raf_callbacks = Object.create(null);
            const ids = Object.keys(callbacks);

            for (let i = 0; i < ids.length; i++) {
                try {
                    callbacks[ids[i]](ts);
                } catch (e) {
                    console.error('RAF callback error:', e);
                }
            }

            // Flush all batched canvas commands at end of frame.
            if (globalThis.__migo_frame_end_all) {
                globalThis.__migo_frame_end_all();
            }
        }
    } catch (e) {
        console.error('RAF loop terminated:', e);
        __rafLoopRunning = false;
    }
}

const setPreferredFramesPerSecond = (fps) => {
    op_set_preferred_fps(Math.max(1, Math.min(60, fps | 0)));
};

export { requestAnimationFrame, cancelAnimationFrame, setPreferredFramesPerSecond };
