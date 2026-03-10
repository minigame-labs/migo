// Architecture: Each UDPSocket instance wraps a Rust UdpSocketResource
// (identified by rid). Events are polled via an async loop calling
// op_udp_next_event, matching the TCP/WebSocket pattern.

import {
    op_udp_bind, op_udp_connect, op_udp_send,
    op_udp_set_ttl, op_udp_next_event, op_udp_close,
} from "ext:core/ops";

// -- UDPSocket class --

class UDPSocket {
    constructor(type) {
        this._type = type || 'udp4';
        this._rid = -1;
        this._bound = false;
        this._closed = false;

        // Per-instance event listeners
        this._closeListeners = [];
        this._errorListeners = [];
        this._listeningListeners = [];
        this._messageListeners = [];
    }

    // -- Control methods --

    bind(port) {
        if (this._bound || this._closed) return -1;

        const bindPort = (port !== undefined && port !== null) ? port : 0;
        let resultPort = -1;

        (async () => {
            try {
                const result = await op_udp_bind(bindPort, this._type);
                this._rid = result.rid;
                this._bound = true;
                resultPort = result.port;

                // Fire listening event
                this._fireListening();

                // Start the event polling loop
                await this._pollEvents();
            } catch (err) {
                this._fireError(err.message || 'bind:fail unknown error');
            }
        })();

        // Return the requested port (or 0 for random).
        // The actual port is available after the listening event.
        return bindPort;
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
        if (!this._bound || this._closed) {
            this._fireError('send:fail socket not bound');
            return;
        }

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

    onListening(listener) {
        if (typeof listener === 'function') this._listeningListeners.push(listener);
    }

    offListening(listener) {
        if (typeof listener === 'function') {
            const i = this._listeningListeners.indexOf(listener);
            if (i !== -1) this._listeningListeners.splice(i, 1);
        } else {
            this._listeningListeners.length = 0;
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

    // -- Internal event dispatch --

    _fireListening() {
        for (let i = 0; i < this._listeningListeners.length; i++) {
            try { this._listeningListeners[i](); } catch (e) {
                console.error('UDPSocket onListening listener error:', e);
            }
        }
    }

    _fireClose() {
        for (let i = 0; i < this._closeListeners.length; i++) {
            try { this._closeListeners[i](); } catch (e) {
                console.error('UDPSocket onClose listener error:', e);
            }
        }
    }

    _fireError(errMsg) {
        const res = { errMsg };
        for (let i = 0; i < this._errorListeners.length; i++) {
            try { this._errorListeners[i](res); } catch (e) {
                console.error('UDPSocket onError listener error:', e);
            }
        }
    }

    _fireMessage(message, remoteInfo, localInfo) {
        const res = { message, remoteInfo, localInfo };
        for (let i = 0; i < this._messageListeners.length; i++) {
            try { this._messageListeners[i](res); } catch (e) {
                console.error('UDPSocket onMessage listener error:', e);
            }
        }
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
                    const buf = new Uint8Array(event.data).buffer;
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
