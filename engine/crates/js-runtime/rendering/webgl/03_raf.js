let __nextRafId = 0;
let __raf_callbacks = Object.create(null);

const requestAnimationFrame = (cb) => {
    const id = ++__nextRafId;
    __raf_callbacks[id] = cb;
    return id;
};

const cancelAnimationFrame = (id) => {
    delete __raf_callbacks[id];
};

const _internalScheduleRaf = () => {
    const ts = performance.now();

    const callbacks = __raf_callbacks;
    __raf_callbacks = Object.create(null);
    const ids = Object.keys(callbacks);

    if (ids.length === 0) return;

    for (let i = 0; i < ids.length; i++) {
        const id = ids[i];
        const cb = callbacks[id];
        try {
            cb(ts);
        } catch (e) {
            console.error('RAF callback error:', e);
        }
    }

    // Flush all batched canvas commands at end of frame
    if (globalThis.__migo_frame_end_all) {
        globalThis.__migo_frame_end_all();
    }
};

export { requestAnimationFrame, cancelAnimationFrame, _internalScheduleRaf };