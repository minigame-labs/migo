// Mouse and Wheel event listeners (PC-only)
//
// These APIs are designed for desktop/PC platforms only.
// Events arrive from host via EvalScript calling the _internalTrigger*
// functions. On Android, these events are not dispatched by the host -
// touch events are used instead.
//
// Status: JS listener infrastructure is complete; host-side dispatch
// (EvalScript or HostCommand) not yet wired on any platform.

import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const _mouseDown = createListenerGroup('onMouseDown');
const _mouseMove = createListenerGroup('onMouseMove');
const _mouseUp = createListenerGroup('onMouseUp');
const _wheel = createListenerGroup('onWheel');

// ---- Mouse Down ----

function onMouseDown(listener) {
    _mouseDown.on(listener);
}

function offMouseDown(listener) {
    _mouseDown.off(listener);
}

function _internalTriggerMouseDown(x, y, button, timeStamp) {
    _mouseDown.trigger({ x: x, y: y, button: button, timeStamp: timeStamp });
}

// ---- Mouse Move ----

function onMouseMove(listener) {
    _mouseMove.on(listener);
}

function offMouseMove(listener) {
    _mouseMove.off(listener);
}

function _internalTriggerMouseMove(x, y, button, timeStamp) {
    _mouseMove.trigger({ x: x, y: y, button: button, timeStamp: timeStamp });
}

// ---- Mouse Up ----

function onMouseUp(listener) {
    _mouseUp.on(listener);
}

function offMouseUp(listener) {
    _mouseUp.off(listener);
}

function _internalTriggerMouseUp(x, y, button, timeStamp) {
    _mouseUp.trigger({ x: x, y: y, button: button, timeStamp: timeStamp });
}

// ---- Wheel ----

function onWheel(listener) {
    _wheel.on(listener);
}

function offWheel(listener) {
    _wheel.off(listener);
}

function _internalTriggerWheel(deltaX, deltaY, deltaZ, timeStamp) {
    _wheel.trigger({ deltaX: deltaX, deltaY: deltaY, deltaZ: deltaZ, timeStamp: timeStamp });
}

export {
    onMouseDown, offMouseDown, _internalTriggerMouseDown,
    onMouseMove, offMouseMove, _internalTriggerMouseMove,
    onMouseUp, offMouseUp, _internalTriggerMouseUp,
    onWheel, offWheel, _internalTriggerWheel,
};
