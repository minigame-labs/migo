import { op_worker_create,
    op_worker_post_message,
    op_worker_recv_message,
    op_worker_recv_error,
    op_worker_terminate } from "ext:core/ops";

let currentWorker = null;
class WorkerInstance {
    #messageListeners = [];
    #errorListeners = [];
    #processKilledListeners = [];
    #terminated = false;
    #ready = false;
    #pendingMessages = [];
    #env;

    constructor(scriptPath, options) {
        this.#env = Object.freeze({
            USER_DATA_PATH: globalThis.__USER_DATA_PATH || "",
        });
        this.#init(scriptPath);
    }

    async #init(scriptPath) {
        try {
            await op_worker_create(scriptPath);
            this.#ready = true;

            // Flush any messages queued before the worker was ready
            for (const msg of this.#pendingMessages) {
                op_worker_post_message(msg);
            }
            this.#pendingMessages.length = 0;

            // Start message and error pump loops
            this.#pumpMessages();
            this.#pumpErrors();
        } catch (e) {
            const listeners = this.#errorListeners;
            for (let i = 0; i < listeners.length; i++) {
                try {
                    listeners[i]({ error: e });
                } catch (_) {}
            }
            // Clean up on creation failure
            currentWorker = null;
        }
    }

    async #pumpMessages() {
        while (!this.#terminated) {
            let json;
            try {
                json = await op_worker_recv_message();
            } catch (_) {
                break;
            }
            if (json === null || json === undefined) break;

            let message;
            try {
                message = JSON.parse(json);
            } catch (_) {
                message = json;
            }

            const listeners = this.#messageListeners;
            for (let i = 0; i < listeners.length; i++) {
                try {
                    listeners[i]({ message });
                } catch (e) {
                    console.error("Worker onMessage listener error:", e);
                }
            }
        }
    }

    async #pumpErrors() {
        while (!this.#terminated) {
            let json;
            try {
                json = await op_worker_recv_error();
            } catch (_) {
                break;
            }
            if (json === null || json === undefined) break;

            let error;
            try {
                error = JSON.parse(json);
            } catch (_) {
                error = { message: json };
            }

            const listeners = this.#errorListeners;
            for (let i = 0; i < listeners.length; i++) {
                try {
                    listeners[i]({ error });
                } catch (e) {
                    console.error("Worker onError listener error:", e);
                }
            }
        }
    }

    get env() {
        return this.#env;
    }

    postMessage(message) {
        if (this.#terminated) {
            throw new Error("Worker has been terminated");
        }
        const json = JSON.stringify(message);
        if (!this.#ready) {
            // Queue until worker thread is ready
            this.#pendingMessages.push(json);
            return;
        }
        op_worker_post_message(json);
    }

    onMessage(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#messageListeners.push(listener);
    }

    onError(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#errorListeners.push(listener);
    }

    onProcessKilled(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#processKilledListeners.push(listener);
    }

    terminate() {
        if (this.#terminated) return;
        this.#terminated = true;

        try {
            op_worker_terminate();
        } catch (_) {}

        currentWorker = null;
    }
}

function createWorker(scriptPath, options) {
    if (currentWorker !== null) {
        throw new Error(
            "Only one worker can exist at a time. Call terminate() first."
        );
    }

    const worker = new WorkerInstance(scriptPath, options);
    currentWorker = worker;
    return worker;
}

export { createWorker };
