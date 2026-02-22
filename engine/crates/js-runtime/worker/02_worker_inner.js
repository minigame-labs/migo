import { core } from "ext:core/mod.js";

const {
    op_worker_inner_post_message,
    op_worker_inner_recv_message,
    op_worker_get_camera_frame_data,
} = core.ops;

const messageListeners = [];
const errorListeners = [];

async function _startMessagePump() {
    while (true) {
        let json;
        try {
            json = await op_worker_inner_recv_message();
        } catch (_) {
            break;
        }
        // null/undefined means Terminate signal received
        if (json === null || json === undefined) break;

        let message;
        try {
            message = JSON.parse(json);
        } catch (_) {
            message = json;
        }

        for (let i = 0; i < messageListeners.length; i++) {
            try {
                messageListeners[i]({ message });
            } catch (e) {
                console.error("Worker onMessage listener error:", e);
            }
        }
    }
}

const worker = {
    postMessage(message) {
        op_worker_inner_post_message(JSON.stringify(message));
    },

    onMessage(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        messageListeners.push(listener);
    },

    onError(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        errorListeners.push(listener);
    },

    testOnProcessKilled() {
        // Debug-only: simulate the system reclaiming the worker process.
        // In a real environment, this would trigger onProcessKilled on the main thread.
        console.warn("[Worker] testOnProcessKilled called - not implemented in this runtime");
    },

    getCameraFrameData() {
        return op_worker_get_camera_frame_data();
    },
};

export { worker, _startMessagePump };
