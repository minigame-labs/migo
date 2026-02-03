const _listeners = [];

function onDeviceOrientationChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offDeviceOrientationChange(listener) {
    if (typeof listener === 'function') {
        const index = _listeners.indexOf(listener);
        if (index !== -1) {
            _listeners.splice(index, 1);
        }
    } else {
        _listeners.length = 0;
    }
}

function _internalTriggerDeviceOrientationChange(value) {
    const data = { value };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onDeviceOrientationChange listener error:', e);
        }
    }
}

export {
    onDeviceOrientationChange,
    offDeviceOrientationChange,
    _internalTriggerDeviceOrientationChange,
};
