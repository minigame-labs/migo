// ==================== Memory Warning Event Listeners ====================

import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const _memoryWarningListeners = createListenerGroup('onMemoryWarning');

function onMemoryWarning(listener) {
    _memoryWarningListeners.on(listener);
}

function offMemoryWarning(listener) {
    _memoryWarningListeners.off(listener);
}

// Called from Rust JsBindings dispatch
function _internalTriggerMemoryWarning(level) {
    _memoryWarningListeners.trigger({ level });
}

export {
    onMemoryWarning,
    offMemoryWarning,
    _internalTriggerMemoryWarning,
};
