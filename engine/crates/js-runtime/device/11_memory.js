// ==================== Memory Warning Event Listeners ====================

const _memoryWarningListeners = [];

function onMemoryWarning(listener) {
    if (typeof listener === 'function') {
        _memoryWarningListeners.push(listener);
    }
}

function offMemoryWarning(listener) {
    if (typeof listener === 'function') {
        const index = _memoryWarningListeners.indexOf(listener);
        if (index !== -1) {
            _memoryWarningListeners.splice(index, 1);
        }
    } else {
        _memoryWarningListeners.length = 0;
    }
}

// Called from Rust JsBindings dispatch
function _internalTriggerMemoryWarning(level) {
    const data = { level };
    for (let i = 0; i < _memoryWarningListeners.length; i++) {
        try { _memoryWarningListeners[i](data); } catch (e) {
            console.error('onMemoryWarning listener error:', e);
        }
    }
}

export {
    onMemoryWarning,
    offMemoryWarning,
    _internalTriggerMemoryWarning,
};
