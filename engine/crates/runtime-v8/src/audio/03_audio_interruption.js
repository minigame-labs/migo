import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const _begin = createListenerGroup('onAudioInterruptionBegin');
const _end = createListenerGroup('onAudioInterruptionEnd');

function onAudioInterruptionBegin(listener) { _begin.on(listener); }
function offAudioInterruptionBegin(listener) { _begin.off(listener); }
function onAudioInterruptionEnd(listener) { _end.on(listener); }
function offAudioInterruptionEnd(listener) { _end.off(listener); }

function _internalTriggerAudioInterruptionBegin() { _begin.trigger(); }
function _internalTriggerAudioInterruptionEnd() { _end.trigger(); }

export {
    onAudioInterruptionBegin,
    offAudioInterruptionBegin,
    onAudioInterruptionEnd,
    offAudioInterruptionEnd,
    _internalTriggerAudioInterruptionBegin,
    _internalTriggerAudioInterruptionEnd,
};
