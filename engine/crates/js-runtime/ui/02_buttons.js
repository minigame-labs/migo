// createUserInfoButton / createGameClubButton / getMenuButtonBoundingClientRect
//
// Minimal mock implementations that return callable objects with the
// show/hide/destroy/onTap interface the game expects.  onTap fires
// immediately with stub user info so the auth flow is not blocked.

import { op_get_menu_button_rect } from "ext:core/ops";

// ---- UserInfoButton --------------------------------------------------------

class UserInfoButton {
    #style;
    #type;
    #text;
    #image;
    #visible = false;
    #destroyed = false;
    #tapListeners = [];

    constructor(options) {
        const opts = options || {};
        this.#type = opts.type || 'text';
        this.#text = opts.text || '';
        this.#image = opts.image || '';
        this.#style = Object.assign({
            left: 0, top: 0, width: 0, height: 0,
            backgroundColor: '', color: '#000000',
            textAlign: 'center', fontSize: 16,
            borderRadius: 0, lineHeight: 0,
        }, opts.style || {});
    }

    get style() { return this.#style; }
    get type() { return this.#type; }
    get text() { return this.#text; }
    get image() { return this.#image; }

    show() {
        if (!this.#destroyed) this.#visible = true;
    }

    hide() {
        if (!this.#destroyed) this.#visible = false;
    }

    destroy() {
        this.#destroyed = true;
        this.#visible = false;
        this.#tapListeners.length = 0;
    }

    onTap(listener) {
        if (typeof listener === 'function' && !this.#destroyed) {
            this.#tapListeners.push(listener);
        }
    }

    offTap(listener) {
        if (typeof listener === 'function') {
            const idx = this.#tapListeners.indexOf(listener);
            if (idx !== -1) this.#tapListeners.splice(idx, 1);
        } else {
            this.#tapListeners.length = 0;
        }
    }

    // Called from host when the user taps the button area.
    // If no host integration exists, the game can still function because
    // onTap listeners are registered but simply won't fire until the host
    // calls _internalTriggerUserInfoButtonTap.
    _fireTap(userInfoJsonOrObj) {
        if (this.#destroyed) return;
        let parsed;
        if (userInfoJsonOrObj && typeof userInfoJsonOrObj === 'object') {
            parsed = userInfoJsonOrObj;
        } else {
            try { parsed = JSON.parse(userInfoJsonOrObj); } catch (_) { parsed = {}; }
        }
        const res = {
            errMsg: 'getUserInfo:ok',
            userInfo: parsed.userInfo || {
                nickName: '',
                avatarUrl: '',
                gender: 0,
                country: '',
                province: '',
                city: '',
                language: 'zh_CN',
            },
            rawData: parsed.rawData || '',
            signature: parsed.signature || '',
            encryptedData: parsed.encryptedData || '',
            iv: parsed.iv || '',
        };
        const listeners = this.#tapListeners.slice();
        for (let i = 0; i < listeners.length; i++) {
            try { listeners[i](res); } catch (e) {
                console.error('UserInfoButton onTap listener error:', e);
            }
        }
    }
}

function createUserInfoButton(options) {
    return new UserInfoButton(options);
}

// ---- GameClubButton --------------------------------------------------------

class GameClubButton {
    #icon;
    #style;
    #visible = false;
    #destroyed = false;

    constructor(options) {
        const opts = options || {};
        this.#icon = opts.icon || 'green';
        this.#style = Object.assign({
            left: 0, top: 0, width: 40, height: 40,
        }, opts.style || {});
    }

    get style() { return this.#style; }
    get icon() { return this.#icon; }

    show() {
        if (!this.#destroyed) this.#visible = true;
    }

    hide() {
        if (!this.#destroyed) this.#visible = false;
    }

    destroy() {
        this.#destroyed = true;
        this.#visible = false;
    }
}

function createGameClubButton(options) {
    return new GameClubButton(options);
}

function getMenuButtonBoundingClientRect() {
    try {
        return JSON.parse(op_get_menu_button_rect());
    } catch (e) {
        return { width: 87, height: 32, top: 4, bottom: 36, left: 278, right: 365 };
    }
}

// ---- FeedbackButton (mock - same interface as UserInfoButton) --------------

class FeedbackButton {
    #style;
    #type;
    #text;
    #image;
    #visible = false;
    #destroyed = false;
    #tapListeners = [];

    constructor(options) {
        const opts = options || {};
        this.#type = opts.type || 'text';
        this.#text = opts.text || '';
        this.#image = opts.image || '';
        this.#style = Object.assign({
            left: 0, top: 0, width: 0, height: 0,
        }, opts.style || {});
    }

    get style() { return this.#style; }
    get type() { return this.#type; }
    get text() { return this.#text; }
    get image() { return this.#image; }

    show() { if (!this.#destroyed) this.#visible = true; }
    hide() { if (!this.#destroyed) this.#visible = false; }
    destroy() {
        this.#destroyed = true;
        this.#visible = false;
        this.#tapListeners.length = 0;
    }

    onTap(listener) {
        if (typeof listener === 'function' && !this.#destroyed) {
            this.#tapListeners.push(listener);
        }
    }

    offTap(listener) {
        if (typeof listener === 'function') {
            const idx = this.#tapListeners.indexOf(listener);
            if (idx !== -1) this.#tapListeners.splice(idx, 1);
        } else {
            this.#tapListeners.length = 0;
        }
    }
}

function createFeedbackButton(options) {
    return new FeedbackButton(options);
}

export {
    createUserInfoButton,
    createGameClubButton,
    getMenuButtonBoundingClientRect,
    createFeedbackButton,
};
