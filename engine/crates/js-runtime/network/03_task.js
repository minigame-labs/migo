import { primordials } from "ext:core/mod.js";
const { TypeError } = primordials;

class NetworkTask {
    constructor(terminator) {
        this._aborted = false;
        this._headersReceivedListeners = [];
        this._terminator = terminator;
    }

    abort() {
        if (this._aborted) {
            return;
        }
        this._aborted = true;
        this._terminator?.abort();
        this._headersReceivedListeners = [];
        this._onCleanup();
    }

    /** Override in subclasses for additional cleanup on abort. */
    _onCleanup() {}

    onHeadersReceived(listener) {
        if (typeof listener !== 'function') {
            throw new TypeError('Listener must be a function');
        }
        if (this._aborted) {
            return;
        }
        this._headersReceivedListeners.push(listener);
    }

    offHeadersReceived(listener) {
        if (listener === undefined) {
            this._headersReceivedListeners = [];
            return;
        }
        if (typeof listener !== 'function') {
            return;
        }
        const index = this._headersReceivedListeners.indexOf(listener);
        if (index !== -1) {
            this._headersReceivedListeners.splice(index, 1);
        }
    }

    _triggerHeadersReceived(headers) {
        if (this._aborted) {
            return;
        }
        for (const listener of this._headersReceivedListeners) {
            try {
                listener(headers);
            } catch (error) {
                console.error('Error in headers received listener:', error);
            }
        }
    }

    toJSON() {
        return {};
    }
}

export { NetworkTask };
