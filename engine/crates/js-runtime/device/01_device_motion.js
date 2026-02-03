const _listeners = [];

function onDeviceMotionChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offDeviceMotionChange(listener) {
    if (typeof listener === 'function') {
        const index = _listeners.indexOf(listener);
        if (index !== -1) {
            _listeners.splice(index, 1);
        }
    } else {
        _listeners.length = 0;
    }
}

function _internalTriggerDeviceMotionChange(alpha, beta, gamma) {
    const data = { alpha, beta, gamma };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onDeviceMotionChange listener error:', e);
        }
    }
}

export {
    onDeviceMotionChange,
    offDeviceMotionChange,
    _internalTriggerDeviceMotionChange,
};
