import { primordials } from "ext:core/mod.js";
import { createListenerGroup } from "ext:host_v8_base/02_async.js";
const { TypeError } = primordials;

class NetworkTask {
    constructor(terminator) {
        this._aborted = false;
        this._headersReceivedListeners = createListenerGroup('Error in headers received');
        this._terminator = terminator;
    }

    abort() {
        if (this._aborted) {
            return;
        }
        this._aborted = true;
        this._terminator?.abort();
        this._headersReceivedListeners.off();
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
        this._headersReceivedListeners.on(listener);
    }

    offHeadersReceived(listener) {
        if (listener !== undefined && typeof listener !== 'function') return;
        this._headersReceivedListeners.off(listener);
    }

    _triggerHeadersReceived(headers) {
        if (this._aborted) {
            return;
        }
        this._headersReceivedListeners.trigger(headers);
    }

    toJSON() {
        return {};
    }
}

export { NetworkTask };
