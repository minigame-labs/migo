// createPageManager
//
// Minimal mock: load() resolves immediately, show/hide/destroy are no-ops,
// on/off implement a basic event emitter so the game code does not crash.

import { createListenerGroup } from "ext:host_v8_base/02_async.js";

class PageManager {
    #listeners = {};
    #destroyed = false;
    #visible = false;
    #loadOptions = null;

    load(options) {
        if (this.#destroyed) {
            return Promise.reject({ errCode: -1, errMsg: 'PageManager already destroyed' });
        }
        const opts = options || {};
        this.#loadOptions = {
            openlink: opts.openlink || '',
            query: opts.query || {},
        };
        return Promise.resolve({ errMsg: 'load:ok' });
    }

    getLoadOptions() {
        return this.#loadOptions;
    }

    show() {
        if (this.#destroyed) return;
        this.#visible = true;
        this._fire('show');
    }

    hide() {
        if (this.#destroyed) return;
        this.#visible = false;
    }

    destroy() {
        if (this.#destroyed) return;
        this.#destroyed = true;
        this.#visible = false;
        this._fire('destroy');
        this.#listeners = {};
    }

    on(event, listener) {
        if (typeof event !== 'string' || typeof listener !== 'function') return;
        if (!this.#listeners[event]) this.#listeners[event] = createListenerGroup('PageManager ' + event);
        this.#listeners[event].on(listener);
    }

    off(event, listener) {
        if (typeof event !== 'string') return;
        const group = this.#listeners[event];
        if (!group) return;
        group.off(listener);
    }

    _fire(event, data) {
        const group = this.#listeners[event];
        if (!group) return;
        group.trigger(data);
    }
}

function createPageManager() {
    return new PageManager();
}

export { createPageManager };
