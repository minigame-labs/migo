// TCP Socket API - wx.createTCPSocket() compatible implementation.
//
// Architecture: Each TCPSocket instance wraps a Rust TcpSocketResource
// (identified by rid). Events are polled via an async loop calling
// op_tcp_next_event, matching the WebSocket pattern.

import { core } from "ext:core/mod.js";
import {
    op_tcp_connect, op_tcp_next_event, op_tcp_write, op_tcp_close,
} from "ext:core/ops";

// -- TCPSocket class --

class TCPSocket {
    constructor(type) {
        this._type = type || 'ipv4';
        this._rid = -1;
        this._closed = false;
        this._connected = false;

        // Per-instance event listeners
        this._connectListeners = [];
        this._closeListeners = [];
        this._errorListeners = [];
        this._messageListeners = [];
        this._bindWifiListeners = [];
    }

    // -- Control methods --

    connect(options = {}) {
        if (this._connected || this._closed) return;

        const {
            address,
            port,
            timeout = 2,
        } = options;

        if (!address || port === undefined) {
            this._fireError('connect:fail missing address or port');
            return;
        }

        (async () => {
            try {
                const result = await op_tcp_connect(address, port, timeout);
                this._rid = result.rid;
                this._connected = true;

                // Fire connect event with address info
                this._fireConnect({
                    remoteInfo: {
                        address: result.remoteAddress,
                        family: result.remoteFamily,
                        port: result.remotePort,
                    },
                    localInfo: {
                        address: result.localAddress,
                        family: result.localFamily,
                        port: result.localPort,
                    },
                });

                // Start the event polling loop
                await this._pollEvents();

            } catch (err) {
                this._fireError(err.message || 'connect:fail unknown error');
                this._doClose();
            }
        })();
    }

    write(data) {
        if (!this._connected || this._closed) return;

        let dataStr = undefined;
        let dataBuf = undefined;

        if (typeof data === 'string') {
            dataStr = data;
        } else if (data instanceof ArrayBuffer) {
            dataBuf = new Uint8Array(data);
        } else if (ArrayBuffer.isView(data)) {
            dataBuf = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        } else {
            this._fireError('write:fail invalid data type');
            return;
        }

        op_tcp_write(this._rid, dataStr, dataBuf).catch((err) => {
            if (!this._closed) {
                this._fireError('write:fail ' + (err.message || err));
            }
        });
    }

    close() {
        if (this._closed) return;
        this._doClose();
    }

    bindWifi(options = {}) {
        // bindWifi requires Android Network.bindSocket() -- not yet implemented.
        // Fire error per wx semantics.
        this._fireError('bindWifi:fail not supported');
    }

    // -- Event listener methods --

    onConnect(listener) {
        if (typeof listener === 'function') this._connectListeners.push(listener);
    }

    offConnect(listener) {
        if (typeof listener === 'function') {
            const i = this._connectListeners.indexOf(listener);
            if (i !== -1) this._connectListeners.splice(i, 1);
        } else {
            this._connectListeners.length = 0;
        }
    }

    onClose(listener) {
        if (typeof listener === 'function') this._closeListeners.push(listener);
    }

    offClose(listener) {
        if (typeof listener === 'function') {
            const i = this._closeListeners.indexOf(listener);
            if (i !== -1) this._closeListeners.splice(i, 1);
        } else {
            this._closeListeners.length = 0;
        }
    }

    onError(listener) {
        if (typeof listener === 'function') this._errorListeners.push(listener);
    }

    offError(listener) {
        if (typeof listener === 'function') {
            const i = this._errorListeners.indexOf(listener);
            if (i !== -1) this._errorListeners.splice(i, 1);
        } else {
            this._errorListeners.length = 0;
        }
    }

    onMessage(listener) {
        if (typeof listener === 'function') this._messageListeners.push(listener);
    }

    offMessage(listener) {
        if (typeof listener === 'function') {
            const i = this._messageListeners.indexOf(listener);
            if (i !== -1) this._messageListeners.splice(i, 1);
        } else {
            this._messageListeners.length = 0;
        }
    }

    onBindWifi(listener) {
        if (typeof listener === 'function') this._bindWifiListeners.push(listener);
    }

    offBindWifi(listener) {
        if (typeof listener === 'function') {
            const i = this._bindWifiListeners.indexOf(listener);
            if (i !== -1) this._bindWifiListeners.splice(i, 1);
        } else {
            this._bindWifiListeners.length = 0;
        }
    }

    // -- Internal event dispatch --

    _fireConnect(res) {
        for (let i = 0; i < this._connectListeners.length; i++) {
            try { this._connectListeners[i](res); } catch (e) {
                console.error('TCPSocket onConnect listener error:', e);
            }
        }
    }

    _fireClose() {
        for (let i = 0; i < this._closeListeners.length; i++) {
            try { this._closeListeners[i](); } catch (e) {
                console.error('TCPSocket onClose listener error:', e);
            }
        }
    }

    _fireError(errMsg) {
        const res = { errMsg };
        for (let i = 0; i < this._errorListeners.length; i++) {
            try { this._errorListeners[i](res); } catch (e) {
                console.error('TCPSocket onError listener error:', e);
            }
        }
    }

    _fireMessage(message, remoteInfo, localInfo) {
        const res = { message, remoteInfo, localInfo };
        for (let i = 0; i < this._messageListeners.length; i++) {
            try { this._messageListeners[i](res); } catch (e) {
                console.error('TCPSocket onMessage listener error:', e);
            }
        }
    }

    _doClose() {
        if (this._closed) return;
        this._closed = true;
        this._connected = false;
        if (this._rid >= 0) {
            try { op_tcp_close(this._rid); } catch (_) { /* ignore */ }
        }
        this._fireClose();
    }

    async _pollEvents() {
        while (!this._closed && this._rid >= 0) {
            let event;
            try {
                event = await op_tcp_next_event(this._rid);
            } catch (err) {
                if (!this._closed) {
                    this._fireError(err.message || 'connection lost');
                    this._doClose();
                }
                return;
            }

            switch (event.type) {
                case 'message': {
                    // Convert the raw byte array to ArrayBuffer for wx compatibility
                    const buf = new Uint8Array(event.data).buffer;
                    this._fireMessage(
                        buf,
                        {
                            address: event.remoteAddress,
                            family: event.remoteFamily,
                            port: event.remotePort,
                        },
                        {
                            address: event.localAddress,
                            family: event.localFamily,
                            port: event.localPort,
                        },
                    );
                    break;
                }
                case 'error':
                    this._fireError(event.errMsg);
                    break;
                case 'close':
                    this._doClose();
                    return;
            }
        }
    }
}

// -- Factory function --

function createTCPSocket(options = {}) {
    const type = options.type || 'ipv4';
    return new TCPSocket(type);
}

export { createTCPSocket };
