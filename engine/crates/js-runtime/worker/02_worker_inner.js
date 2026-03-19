import { op_worker_inner_post_message,
    op_worker_inner_recv_message } from "ext:core/ops";
import * as request from "ext:host_v8_network/04_request.js";
import * as download from "ext:host_v8_network/05_download.js";
import * as upload from "ext:host_v8_network/06_upload.js";
import * as websocket from "ext:host_v8_network/07_websocket.js";
import * as innerAudio from "ext:host_v8_audio/02_inner_audio_context.js";
import * as fileManager from "ext:host_v8_file/02_file_manager.js";
import * as envApi from "ext:host_v8_env/00_env.js";

const messageListeners = [];

async function _startMessagePump() {
    console.log("[Worker-JS] message pump started, listeners:", messageListeners.length);
    while (true) {
        let json;
        try {
            json = await op_worker_inner_recv_message();
        } catch (e) {
            console.error("[Worker-JS] recv error:", e);
            break;
        }
        // null/undefined means Terminate signal received
        if (json === null || json === undefined) {
            console.log("[Worker-JS] received null/terminate, exiting pump");
            break;
        }

        console.log("[Worker-JS] received message, listeners:", messageListeners.length);

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
                console.error("[Worker-JS] onMessage listener error:", e);
            }
        }
    }
}

const worker = {
    postMessage(message) {
        console.log("[Worker-JS] postMessage to main:", JSON.stringify(message));
        op_worker_inner_post_message(JSON.stringify(message));
    },

    onMessage(listener) {
        if (typeof listener !== "function") {
            throw new TypeError("listener must be a function");
        }
        messageListeners.push(listener);
        console.log("[Worker-JS] onMessage listener registered, total:", messageListeners.length);
    },

    connectSocket: websocket.connectSocket,
    createInnerAudioContext: innerAudio.createInnerAudioContext,
    downloadFile: download.downloadFile,
    env: envApi.env,
    getFileSystemManager() {
        return fileManager.getFileSystemManager();
    },
    request: request.request,
    uploadFile: upload.uploadFile,
};

export { worker, _startMessagePump };
