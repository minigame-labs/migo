// Architecture: Each UDPSocket instance wraps a Rust UdpSocketResource
// (identified by rid). Events are polled via an async loop calling
// op_udp_next_event, matching the TCP/WebSocket pattern.

import {
    op_udp_bind, op_udp_connect, op_udp_send,
    op_udp_set_ttl, op_udp_next_event, op_udp_close,
} from "ext:core/ops";
import { createListenerGroup } from "ext:host_v8_base/02_async.js";
import { toExactArrayBuffer } from "ext:host_v8_network/00_binary.js";

// -- UDPSocket class --

class UDPSocket {
    constructor(type) {
        this._type = type || 'udp4';
        this._rid = -1;
        this._bound = false;
        this._closed = false;

        // Per-instance event listeners
        this._closeListeners = createListenerGroup('UDPSocket onClose');
        this._errorListeners = createListenerGroup('UDPSocket onError');
        this._listeningListeners = createListenerGroup('UDPSocket onListening');
        this._messageListeners = createListenerGroup('UDPSocket onMessage');
    }

    // -- Control methods --

    bind(port) {
        if (this._bound || this._closed) return -1;

        const bindPort = (port !== undefined && port !== null) ? port : 0;

        try {
            const result = op_udp_bind(bindPort, this._type);
            this._rid = result.rid;
            this._bound = true;

            // Fire listening event
            this._fireListening();

            // Start the event polling loop in background
            this._pollEvents();

            return result.port;
        } catch (err) {
            this._fireError(err.message || 'bind:fail unknown error');
            return -1;
        }
    }

    setTTL(ttl) {
        if (!this._bound || this._closed) return;
        if (ttl < 0 || ttl > 255) {
            this._fireError('setTTL:fail ttl must be between 0 and 255');
            return;
        }
        try {
            op_udp_set_ttl(this._rid, ttl);
        } catch (err) {
            this._fireError(err.message || 'setTTL:fail');
        }
    }

    send(options = {}) {
        const { address, port, message, offset, length, setBroadcast } = options;

        if (!address) {
            this._fireError('send:fail missing address');
            return;
        }
        if (port === undefined || port === null) {
            this._fireError('send:fail missing port');
            return;
        }

        let dataStr = undefined;
        let dataBuf = undefined;
        let dataOffset = offset || 0;
        let dataLength = length || 0;

        if (typeof message === 'string') {
            dataStr = message;
        } else if (message instanceof ArrayBuffer) {
            dataBuf = new Uint8Array(message);
            if (!dataLength) dataLength = dataBuf.byteLength - dataOffset;
        } else if (ArrayBuffer.isView(message)) {
            dataBuf = new Uint8Array(message.buffer, message.byteOffset, message.byteLength);
            if (!dataLength) dataLength = dataBuf.byteLength - dataOffset;
        } else {
            this._fireError('send:fail invalid message type');
            return;
        }

        if (!this._bound || this._closed) {
            this._fireError('send:fail socket not bound');
            return;
        }

        op_udp_send(
            this._rid, address, port,
            dataStr, dataBuf,
            dataOffset, dataLength,
            setBroadcast || false,
        ).catch((err) => {
            if (!this._closed) {
                this._fireError('send:fail ' + (err.message || err));
            }
        });
    }

    connect(options = {}) {
        if (!this._bound || this._closed) {
            this._fireError('connect:fail socket not bound');
            return;
        }

        const { address, port } = options;

        if (!address || port === undefined) {
            this._fireError('connect:fail missing address or port');
            return;
        }

        op_udp_connect(this._rid, address, port).catch((err) => {
            if (!this._closed) {
                this._fireError('connect:fail ' + (err.message || err));
            }
        });
    }

    write(options = {}) {
        this.send(options);
    }

    close() {
        if (this._closed) return;
        this._doClose();
    }

    // -- Event listener methods --

    onClose(listener) {
        this._closeListeners.on(listener);
    }

    offClose(listener) {
        this._closeListeners.off(listener);
    }

    onError(listener) {
        this._errorListeners.on(listener);
    }

    offError(listener) {
        this._errorListeners.off(listener);
    }

    onListening(listener) {
        this._listeningListeners.on(listener);
    }

    offListening(listener) {
        this._listeningListeners.off(listener);
    }

    onMessage(listener) {
        this._messageListeners.on(listener);
    }

    offMessage(listener) {
        this._messageListeners.off(listener);
    }

    // -- Internal event dispatch --

    _fireListening() {
        this._listeningListeners.trigger();
    }

    _fireClose() {
        this._closeListeners.trigger();
    }

    _fireError(errMsg) {
        this._errorListeners.trigger({ errMsg });
    }

    _fireMessage(message, remoteInfo, localInfo) {
        this._messageListeners.trigger({ message, remoteInfo, localInfo });
    }

    _doClose() {
        if (this._closed) return;
        this._closed = true;
        this._bound = false;
        if (this._rid >= 0) {
            try { op_udp_close(this._rid); } catch (_) { /* ignore */ }
        }
        this._fireClose();
    }

    async _pollEvents() {
        while (!this._closed && this._rid >= 0) {
            let event;
            try {
                event = await op_udp_next_event(this._rid);
            } catch (err) {
                if (!this._closed) {
                    this._fireError(err.message || 'receive error');
                    this._doClose();
                }
                return;
            }

            switch (event.type) {
                case 'message': {
                    // event.data is an exact-length Uint8Array (external
                    // backing); hand its buffer to the callback without a copy.
                    const buf = toExactArrayBuffer(event.data);
                    this._fireMessage(
                        buf,
                        {
                            address: event.remoteAddress,
                            family: event.remoteFamily,
                            port: event.remotePort,
                            size: event.size,
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

function createUDPSocket(options = {}) {
    const type = options.type || 'udp4';
    return new UDPSocket(type);
}

export { createUDPSocket };
