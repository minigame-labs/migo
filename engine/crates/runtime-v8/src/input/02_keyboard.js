import { op_show_keyboard, op_hide_keyboard, op_update_keyboard } from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

// ==================== Soft keyboard event listeners ====================

const _inputListeners = createListenerGroup('onKeyboardInput');
const _heightChangeListeners = createListenerGroup('onKeyboardHeightChange');
const _confirmListeners = createListenerGroup('onKeyboardConfirm');
const _completeListeners = createListenerGroup('onKeyboardComplete');

// ---- onKeyboardInput / offKeyboardInput ----

function onKeyboardInput(listener) {
    _inputListeners.on(listener);
}

function offKeyboardInput(listener) {
    _inputListeners.off(listener);
}

// ---- onKeyboardHeightChange / offKeyboardHeightChange ----

function onKeyboardHeightChange(listener) {
    _heightChangeListeners.on(listener);
}

function offKeyboardHeightChange(listener) {
    _heightChangeListeners.off(listener);
}

// ---- onKeyboardConfirm / offKeyboardConfirm ----

function onKeyboardConfirm(listener) {
    _confirmListeners.on(listener);
}

function offKeyboardConfirm(listener) {
    _confirmListeners.off(listener);
}

// ---- onKeyboardComplete / offKeyboardComplete ----

function onKeyboardComplete(listener) {
    _completeListeners.on(listener);
}

function offKeyboardComplete(listener) {
    _completeListeners.off(listener);
}

// ==================== Internal trigger functions (called from native) ====================

function _internalTriggerKeyboardInput(value) {
    _inputListeners.trigger({ value });
}

function _internalTriggerKeyboardHeightChange(height) {
    _heightChangeListeners.trigger({ height });
}

function _internalTriggerKeyboardConfirm(value) {
    _confirmListeners.trigger({ value });
}

function _internalTriggerKeyboardComplete(value) {
    _completeListeners.trigger({ value });
}

// ==================== PC keyboard event listeners ====================

const _keyDownListeners = createListenerGroup('onKeyDown');
const _keyUpListeners = createListenerGroup('onKeyUp');

function onKeyDown(listener) {
    _keyDownListeners.on(listener);
}

function offKeyDown(listener) {
    _keyDownListeners.off(listener);
}

function onKeyUp(listener) {
    _keyUpListeners.on(listener);
}

function offKeyUp(listener) {
    _keyUpListeners.off(listener);
}

function _internalTriggerKeyDown(key, code, timeStamp) {
    _keyDownListeners.trigger({ key, code, timeStamp });
}

function _internalTriggerKeyUp(key, code, timeStamp) {
    _keyUpListeners.trigger({ key, code, timeStamp });
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
