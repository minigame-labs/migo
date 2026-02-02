import { core } from "ext:core/mod.js";
import {
    op_ws_create, op_ws_next_event, op_ws_send, op_ws_close,
} from "ext:core/ops";

// -- SocketTask --

class SocketTask {
    constructor(rid) {
        this._rid = rid;
        this._closed = false;
        this._openListeners = [];
        this._messageListeners = [];
        this._errorListeners = [];
        this._closeListeners = [];
    }

    send(options = {}) {
        const { data, success, fail, complete } = options;
        if (this._closed) {
            const err = { errMsg: "sendSocketMessage:fail WebSocket is closed" };
            if (typeof fail === 'function') fail(err);
            if (typeof complete === 'function') complete(err);
            return;
        }

        let dataStr = undefined;
        let dataBuf = undefined;

        if (typeof data === 'string') {
            dataStr = data;
        } else if (data instanceof ArrayBuffer) {
            dataBuf = new Uint8Array(data);
        } else if (ArrayBuffer.isView(data)) {
            dataBuf = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        } else {
            const err = { errMsg: "sendSocketMessage:fail invalid data type" };
            if (typeof fail === 'function') fail(err);
            if (typeof complete === 'function') complete(err);
            return;
        }

        op_ws_send(this._rid, dataStr, dataBuf).then(() => {
            const res = { errMsg: "sendSocketMessage:ok" };
            if (typeof success === 'function') success(res);
            if (typeof complete === 'function') complete(res);
        }).catch((err) => {
            const res = { errMsg: "sendSocketMessage:fail " + err.message };
            if (typeof fail === 'function') fail(res);
            if (typeof complete === 'function') complete(res);
        });
    }

    close(options = {}) {
        const { code = 1000, reason = "", success, fail, complete } = options;
        if (this._closed) {
            const res = { errMsg: "closeSocket:ok" };
            if (typeof success === 'function') success(res);
            if (typeof complete === 'function') complete(res);
            return;
        }

        op_ws_close(this._rid, code, reason).then(() => {
            const res = { errMsg: "closeSocket:ok" };
            if (typeof success === 'function') success(res);
            if (typeof complete === 'function') complete(res);
        }).catch((err) => {
            const res = { errMsg: "closeSocket:fail " + err.message };
            if (typeof fail === 'function') fail(res);
            if (typeof complete === 'function') complete(res);
        });
    }

    onOpen(callback) {
        if (typeof callback === 'function') this._openListeners.push(callback);
    }

    onMessage(callback) {
        if (typeof callback === 'function') this._messageListeners.push(callback);
    }

    onError(callback) {
        if (typeof callback === 'function') this._errorListeners.push(callback);
    }

    onClose(callback) {
        if (typeof callback === 'function') this._closeListeners.push(callback);
    }

    // -- Internal event dispatch --

    _fireOpen(header) {
        for (const cb of this._openListeners) {
            try { cb({ header }); } catch (e) { console.error('SocketTask onOpen error:', e); }
        }
    }

    _fireMessage(data) {
        for (const cb of this._messageListeners) {
            try { cb({ data }); } catch (e) { console.error('SocketTask onMessage error:', e); }
        }
    }

    _fireError(errMsg) {
        for (const cb of this._errorListeners) {
            try { cb({ errMsg }); } catch (e) { console.error('SocketTask onError error:', e); }
        }
    }

    _fireClose(code, reason) {
        this._closed = true;
        core.tryClose(this._rid);
        for (const cb of this._closeListeners) {
            try { cb({ code, reason }); } catch (e) { console.error('SocketTask onClose error:', e); }
        }
    }
}

// -- Event polling loop --

async function _pollEvents(task) {
    while (!task._closed) {
        let event;
        try {
            event = await op_ws_next_event(task._rid);
        } catch (err) {
            if (!task._closed) {
                task._fireError(err.message || "connection lost");
                task._fireClose(1006, "");
            }
            return;
        }

        switch (event.type) {
            case "message":
                if (event.isBinary) {
                    const buf = new Uint8Array(event.dataBin).buffer;
                    task._fireMessage(buf);
                } else {
                    task._fireMessage(event.dataStr);
                }
                break;

            case "error":
                task._fireError(event.errMsg);
                break;

            case "close":
                task._fireClose(event.code, event.reason);
                return;
        }
    }
}

// -- Global socket state --

let _globalSocket = null;
const _globalOpenListeners = [];
const _globalMessageListeners = [];
const _globalErrorListeners = [];
const _globalCloseListeners = [];

// -- connectSocket --

function connectSocket(options = {}) {
    const {
        url,
        header = {},
        protocols = [],
        tcpNoDelay = false,
        perMessageDeflate = false,
        timeout = 60000,
        success,
        fail,
        complete,
    } = options;

    if (!url || typeof url !== 'string') {
        const err = { errMsg: "connectSocket:fail invalid url" };
        if (typeof fail === 'function') queueMicrotask(() => fail(err));
        if (typeof complete === 'function') queueMicrotask(() => complete(err));
        return new SocketTask(-1);
    }

    const headerList = [];
    for (const key of Object.keys(header)) {
        headerList.push([key, String(header[key])]);
    }

    const task = new SocketTask(-1);

    // Set as global socket
    _globalSocket = task;

    // Bridge global listeners to this task
    task.onOpen((res) => {
        for (const cb of _globalOpenListeners) {
            try { cb(res); } catch (e) { console.error(e); }
        }
    });
    task.onMessage((res) => {
        for (const cb of _globalMessageListeners) {
            try { cb(res); } catch (e) { console.error(e); }
        }
    });
    task.onError((res) => {
        for (const cb of _globalErrorListeners) {
            try { cb(res); } catch (e) { console.error(e); }
        }
    });
    task.onClose((res) => {
        for (const cb of _globalCloseListeners) {
            try { cb(res); } catch (e) { console.error(e); }
        }
        if (_globalSocket === task) {
            _globalSocket = null;
        }
    });

    // Async connection
    (async () => {
        try {
            const result = await op_ws_create(url, protocols, headerList);
            task._rid = result.rid;

            const res = { errMsg: "connectSocket:ok" };
            if (typeof success === 'function') success(res);
            if (typeof complete === 'function') complete(res);

            // Fire onOpen with response headers
            task._fireOpen({
                "Sec-WebSocket-Protocol": result.protocol,
                "Sec-WebSocket-Extensions": result.extensions,
            });

            // Start event polling loop
            await _pollEvents(task);

        } catch (err) {
            const res = { errMsg: "connectSocket:fail " + (err.message || err) };
            if (typeof fail === 'function') fail(res);
            if (typeof complete === 'function') complete(res);
            task._fireError(res.errMsg);
            task._fireClose(1006, "");
        }
    })();

    return task;
}

// -- Global convenience APIs --

function sendSocketMessage(options = {}) {
    if (!_globalSocket || _globalSocket._closed) {
        const err = { errMsg: "sendSocketMessage:fail WebSocket is not connected" };
        if (typeof options.fail === 'function') options.fail(err);
        if (typeof options.complete === 'function') options.complete(err);
        return;
    }
    _globalSocket.send(options);
}

function closeSocket(options = {}) {
    if (!_globalSocket || _globalSocket._closed) {
        const err = { errMsg: "closeSocket:fail WebSocket is not connected" };
        if (typeof options.fail === 'function') options.fail(err);
        if (typeof options.complete === 'function') options.complete(err);
        return;
    }
    _globalSocket.close(options);
}

function onSocketOpen(callback) {
    if (typeof callback === 'function') _globalOpenListeners.push(callback);
}

function offSocketOpen(callback) {
    if (callback === undefined) {
        _globalOpenListeners.length = 0;
        return;
    }
    const idx = _globalOpenListeners.indexOf(callback);
    if (idx !== -1) _globalOpenListeners.splice(idx, 1);
}

function onSocketMessage(callback) {
    if (typeof callback === 'function') _globalMessageListeners.push(callback);
}

function offSocketMessage(callback) {
    if (callback === undefined) {
        _globalMessageListeners.length = 0;
        return;
    }
    const idx = _globalMessageListeners.indexOf(callback);
    if (idx !== -1) _globalMessageListeners.splice(idx, 1);
}

function onSocketError(callback) {
    if (typeof callback === 'function') _globalErrorListeners.push(callback);
}

function offSocketError(callback) {
    if (callback === undefined) {
        _globalErrorListeners.length = 0;
        return;
    }
    const idx = _globalErrorListeners.indexOf(callback);
    if (idx !== -1) _globalErrorListeners.splice(idx, 1);
}

function onSocketClose(callback) {
    if (typeof callback === 'function') _globalCloseListeners.push(callback);
}

function offSocketClose(callback) {
    if (callback === undefined) {
        _globalCloseListeners.length = 0;
        return;
    }
    const idx = _globalCloseListeners.indexOf(callback);
    if (idx !== -1) _globalCloseListeners.splice(idx, 1);
}

export {
    connectSocket,
    sendSocketMessage,
    closeSocket,
    onSocketOpen,
    offSocketOpen,
    onSocketMessage,
    offSocketMessage,
    onSocketError,
    offSocketError,
    onSocketClose,
    offSocketClose,
};
