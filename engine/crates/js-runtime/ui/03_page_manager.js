// createPageManager
//
// Minimal mock: load() resolves immediately, show/hide/destroy are no-ops,
// on/off implement a basic event emitter so the game code does not crash.

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
        if (!this.#listeners[event]) this.#listeners[event] = [];
        this.#listeners[event].push(listener);
    }

    off(event, listener) {
        if (typeof event !== 'string') return;
        const list = this.#listeners[event];
        if (!list) return;
        if (typeof listener === 'function') {
            const idx = list.indexOf(listener);
            if (idx !== -1) list.splice(idx, 1);
        } else {
            // off('show') without listener -> remove all for that event
            this.#listeners[event] = [];
        }
    }

    _fire(event, data) {
        const list = this.#listeners[event];
        if (!list || list.length === 0) return;
        const snapshot = list.slice();
        for (let i = 0; i < snapshot.length; i++) {
            try { snapshot[i](data); } catch (e) {
                console.error('PageManager ' + event + ' listener error:', e);
            }
        }
    }
}

function createPageManager() {
    return new PageManager();
}

export { createPageManager };
