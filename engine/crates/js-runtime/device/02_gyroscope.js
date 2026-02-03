const _listeners = [];

function onGyroscopeChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offGyroscopeChange(listener) {
    if (typeof listener === 'function') {
        const index = _listeners.indexOf(listener);
        if (index !== -1) {
            _listeners.splice(index, 1);
        }
    } else {
        _listeners.length = 0;
    }
}

function _internalTriggerGyroscopeChange(x, y, z) {
    const data = { x, y, z };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onGyroscopeChange listener error:', e);
        }
    }
}

export {
    onGyroscopeChange,
    offGyroscopeChange,
    _internalTriggerGyroscopeChange,
};
