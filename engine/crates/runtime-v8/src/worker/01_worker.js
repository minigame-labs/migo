import { op_worker_create,
    op_worker_post_message,
    op_worker_recv_message,
    op_worker_recv_error,
    op_worker_terminate } from "ext:core/ops";
import { createListenerGroup } from "ext:host_v8_base/02_async.js";

let currentWorker = null;
class WorkerInstance {
    #messageListeners = createListenerGroup("[Main-Worker] onMessage");
    #errorListeners = createListenerGroup("Worker onError");
    #processKilledListeners = createListenerGroup("Worker onProcessKilled");
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
            console.log("[Main-Worker] creating worker for:", scriptPath);
            await op_worker_create(scriptPath);
            this.#ready = true;
            console.log("[Main-Worker] worker created, ready=true, pending msgs:", this.#pendingMessages.length);

            // Flush any messages queued before the worker was ready
            for (const msg of this.#pendingMessages) {
                console.log("[Main-Worker] flushing pending message:", msg.length, "bytes");
                op_worker_post_message(msg);
            }
            this.#pendingMessages.length = 0;

            // Start error pump first so worker errors are caught immediately
            console.log("[Main-Worker] starting error and message pumps");
            this.#pumpErrors();
            this.#pumpMessages();
        } catch (e) {
            this.#errorListeners.trigger({ error: e });
            // Clean up on creation failure
            currentWorker = null;
        }
    }

    async #pumpMessages() {
        console.log("[Main-Worker] pumpMessages started, listeners:", this.#messageListeners.size());
        while (!this.#terminated) {
            let json;
            try {
                json = await op_worker_recv_message();
            } catch (e) {
                console.error("[Main-Worker] pumpMessages recv error:", e);
                break;
            }
            if (json === null || json === undefined) {
                console.log("[Main-Worker] pumpMessages received null, exiting");
                break;
            }

            console.log("[Main-Worker] received from worker:", json.length, "bytes, listeners:", this.#messageListeners.size());

            let message;
            try {
                message = JSON.parse(json);
            } catch (_) {
                message = json;
            }

            this.#messageListeners.trigger({ message });
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

            this.#errorListeners.trigger({ error });
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
        console.log("[Main-Worker] postMessage to worker:", json.length, "bytes, ready:", this.#ready);
        if (!this.#ready) {
            // Queue until worker thread is ready
            console.log("[Main-Worker] worker not ready, queueing message");
            this.#pendingMessages.push(json);
            return;
        }
        op_worker_post_message(json);
    }

    onMessage(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#messageListeners.on(listener);
    }

    onError(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#errorListeners.on(listener);
    }

    onProcessKilled(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        this.#processKilledListeners.on(listener);
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
