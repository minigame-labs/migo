// Mouse and Wheel event listeners (PC-only)
//
// These APIs are designed for desktop/PC platforms only.
// Events arrive from host via EvalScript calling the _internalTrigger*
// functions. On Android, these events are not dispatched by the host -
// touch events are used instead.
//
// Status: JS listener infrastructure is complete; host-side dispatch
// (EvalScript or HostCommand) not yet wired on any platform.

// ---- Mouse Down ----

var _mouseDownListeners = [];

function onMouseDown(listener) {
    if (typeof listener === 'function') {
        _mouseDownListeners.push(listener);
    }
}

function offMouseDown(listener) {
    if (typeof listener === 'function') {
        var index = _mouseDownListeners.indexOf(listener);
        if (index !== -1) {
            _mouseDownListeners.splice(index, 1);
        }
    } else {
        _mouseDownListeners.length = 0;
    }
}

function _internalTriggerMouseDown(x, y, button, timeStamp) {
    var data = { x: x, y: y, button: button, timeStamp: timeStamp };
    for (var i = 0; i < _mouseDownListeners.length; i++) {
        try { _mouseDownListeners[i](data); } catch (e) {
            console.error('onMouseDown listener error:', e);
        }
    }
}

// ---- Mouse Move ----

var _mouseMoveListeners = [];

function onMouseMove(listener) {
    if (typeof listener === 'function') {
        _mouseMoveListeners.push(listener);
    }
}

function offMouseMove(listener) {
    if (typeof listener === 'function') {
        var index = _mouseMoveListeners.indexOf(listener);
        if (index !== -1) {
            _mouseMoveListeners.splice(index, 1);
        }
    } else {
        _mouseMoveListeners.length = 0;
    }
}

function _internalTriggerMouseMove(x, y, button, timeStamp) {
    var data = { x: x, y: y, button: button, timeStamp: timeStamp };
    for (var i = 0; i < _mouseMoveListeners.length; i++) {
        try { _mouseMoveListeners[i](data); } catch (e) {
            console.error('onMouseMove listener error:', e);
        }
    }
}

// ---- Mouse Up ----

var _mouseUpListeners = [];

function onMouseUp(listener) {
    if (typeof listener === 'function') {
        _mouseUpListeners.push(listener);
    }
}

function offMouseUp(listener) {
    if (typeof listener === 'function') {
        var index = _mouseUpListeners.indexOf(listener);
        if (index !== -1) {
            _mouseUpListeners.splice(index, 1);
        }
    } else {
        _mouseUpListeners.length = 0;
    }
}

function _internalTriggerMouseUp(x, y, button, timeStamp) {
    var data = { x: x, y: y, button: button, timeStamp: timeStamp };
    for (var i = 0; i < _mouseUpListeners.length; i++) {
        try { _mouseUpListeners[i](data); } catch (e) {
            console.error('onMouseUp listener error:', e);
        }
    }
}

// ---- Wheel ----

var _wheelListeners = [];

function onWheel(listener) {
    if (typeof listener === 'function') {
        _wheelListeners.push(listener);
    }
}

function offWheel(listener) {
    if (typeof listener === 'function') {
        var index = _wheelListeners.indexOf(listener);
        if (index !== -1) {
            _wheelListeners.splice(index, 1);
        }
    } else {
        _wheelListeners.length = 0;
    }
}

function _internalTriggerWheel(deltaX, deltaY, deltaZ, timeStamp) {
    var data = { deltaX: deltaX, deltaY: deltaY, deltaZ: deltaZ, timeStamp: timeStamp };
    for (var i = 0; i < _wheelListeners.length; i++) {
        try { _wheelListeners[i](data); } catch (e) {
            console.error('onWheel listener error:', e);
        }
    }
}

export {
    onMouseDown, offMouseDown, _internalTriggerMouseDown,
    onMouseMove, offMouseMove, _internalTriggerMouseMove,
    onMouseUp, offMouseUp, _internalTriggerMouseUp,
    onWheel, offWheel, _internalTriggerWheel,
};
