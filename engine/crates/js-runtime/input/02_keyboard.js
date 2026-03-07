import { op_show_keyboard, op_hide_keyboard, op_update_keyboard } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

// ==================== Soft keyboard event listeners ====================

const _inputListeners = [];
const _heightChangeListeners = [];
const _confirmListeners = [];
const _completeListeners = [];

// ---- onKeyboardInput / offKeyboardInput ----

function onKeyboardInput(listener) {
    if (typeof listener === 'function') {
        _inputListeners.push(listener);
    }
}

function offKeyboardInput(listener) {
    if (typeof listener === 'function') {
        const index = _inputListeners.indexOf(listener);
        if (index !== -1) {
            _inputListeners.splice(index, 1);
        }
    } else {
        _inputListeners.length = 0;
    }
}

// ---- onKeyboardHeightChange / offKeyboardHeightChange ----

function onKeyboardHeightChange(listener) {
    if (typeof listener === 'function') {
        _heightChangeListeners.push(listener);
    }
}

function offKeyboardHeightChange(listener) {
    if (typeof listener === 'function') {
        const index = _heightChangeListeners.indexOf(listener);
        if (index !== -1) {
            _heightChangeListeners.splice(index, 1);
        }
    } else {
        _heightChangeListeners.length = 0;
    }
}

// ---- onKeyboardConfirm / offKeyboardConfirm ----

function onKeyboardConfirm(listener) {
    if (typeof listener === 'function') {
        _confirmListeners.push(listener);
    }
}

function offKeyboardConfirm(listener) {
    if (typeof listener === 'function') {
        const index = _confirmListeners.indexOf(listener);
        if (index !== -1) {
            _confirmListeners.splice(index, 1);
        }
    } else {
        _confirmListeners.length = 0;
    }
}

// ---- onKeyboardComplete / offKeyboardComplete ----

function onKeyboardComplete(listener) {
    if (typeof listener === 'function') {
        _completeListeners.push(listener);
    }
}

function offKeyboardComplete(listener) {
    if (typeof listener === 'function') {
        const index = _completeListeners.indexOf(listener);
        if (index !== -1) {
            _completeListeners.splice(index, 1);
        }
    } else {
        _completeListeners.length = 0;
    }
}

// ==================== Internal trigger functions (called from native) ====================

function _internalTriggerKeyboardInput(value) {
    const data = { value };
    for (let i = 0; i < _inputListeners.length; i++) {
        try { _inputListeners[i](data); } catch (e) {
            console.error('onKeyboardInput listener error:', e);
        }
    }
}

function _internalTriggerKeyboardHeightChange(height) {
    const data = { height };
    for (let i = 0; i < _heightChangeListeners.length; i++) {
        try { _heightChangeListeners[i](data); } catch (e) {
            console.error('onKeyboardHeightChange listener error:', e);
        }
    }
}

function _internalTriggerKeyboardConfirm(value) {
    const data = { value };
    for (let i = 0; i < _confirmListeners.length; i++) {
        try { _confirmListeners[i](data); } catch (e) {
            console.error('onKeyboardConfirm listener error:', e);
        }
    }
}

function _internalTriggerKeyboardComplete(value) {
    const data = { value };
    for (let i = 0; i < _completeListeners.length; i++) {
        try { _completeListeners[i](data); } catch (e) {
            console.error('onKeyboardComplete listener error:', e);
        }
    }
}

// ==================== PC keyboard event listeners ====================

const _keyDownListeners = [];
const _keyUpListeners = [];

function onKeyDown(listener) {
    if (typeof listener === 'function') {
        _keyDownListeners.push(listener);
    }
}

function offKeyDown(listener) {
    if (typeof listener === 'function') {
        const index = _keyDownListeners.indexOf(listener);
        if (index !== -1) {
            _keyDownListeners.splice(index, 1);
        }
    } else {
        _keyDownListeners.length = 0;
    }
}

function onKeyUp(listener) {
    if (typeof listener === 'function') {
        _keyUpListeners.push(listener);
    }
}

function offKeyUp(listener) {
    if (typeof listener === 'function') {
        const index = _keyUpListeners.indexOf(listener);
        if (index !== -1) {
            _keyUpListeners.splice(index, 1);
        }
    } else {
        _keyUpListeners.length = 0;
    }
}

function _internalTriggerKeyDown(key, code, timeStamp) {
    const data = { key, code, timeStamp };
    for (let i = 0; i < _keyDownListeners.length; i++) {
        try { _keyDownListeners[i](data); } catch (e) {
            console.error('onKeyDown listener error:', e);
        }
    }
}

function _internalTriggerKeyUp(key, code, timeStamp) {
    const data = { key, code, timeStamp };
    for (let i = 0; i < _keyUpListeners.length; i++) {
        try { _keyUpListeners[i](data); } catch (e) {
            console.error('onKeyUp listener error:', e);
        }
    }
}

// ==================== Async control APIs ====================

function showKeyboard(options = {}) {
    const { defaultValue, maxLength, multiple, confirmHold, confirmType, keyboardType } = options;
    return wrapAsync('showKeyboard', function () {
        op_show_keyboard(JSON.stringify({
            defaultValue: defaultValue !== undefined ? defaultValue : '',
            maxLength: maxLength !== undefined ? maxLength : 140,
            multiple: multiple !== undefined ? multiple : false,
            confirmHold: confirmHold !== undefined ? confirmHold : false,
            confirmType: confirmType !== undefined ? confirmType : 'done',
            keyboardType: keyboardType !== undefined ? keyboardType : 'text',
        }));
    }, options);
}

function hideKeyboard(options = {}) {
    return wrapAsync('hideKeyboard', function () {
        op_hide_keyboard();
    }, options);
}

function updateKeyboard(options = {}) {
    const { value } = options;
    return wrapAsync('updateKeyboard', function () {
        if (value === undefined) {
            throw new Error('value is required');
        }
        op_update_keyboard(value);
    }, options);
}

export {
    // Soft keyboard events
    onKeyboardInput,
    offKeyboardInput,
    onKeyboardHeightChange,
    offKeyboardHeightChange,
    onKeyboardConfirm,
    offKeyboardConfirm,
    onKeyboardComplete,
    offKeyboardComplete,
    // Internal triggers (called from native dispatch)
    _internalTriggerKeyboardInput,
    _internalTriggerKeyboardHeightChange,
    _internalTriggerKeyboardConfirm,
    _internalTriggerKeyboardComplete,
    // PC keyboard events
    onKeyDown,
    offKeyDown,
    onKeyUp,
    offKeyUp,
    _internalTriggerKeyDown,
    _internalTriggerKeyUp,
    // Control APIs
    showKeyboard,
    hideKeyboard,
    updateKeyboard,
};
