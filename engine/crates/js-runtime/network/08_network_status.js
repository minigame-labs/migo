const _listeners = [];

function onNetworkStatusChange(listener) {
    if (typeof listener === 'function') {
        _listeners.push(listener);
    }
}

function offNetworkStatusChange(listener) {
    if (listener === undefined) {
        _listeners.length = 0;
        return;
    }
    const idx = _listeners.indexOf(listener);
    if (idx !== -1) {
        _listeners.splice(idx, 1);
    }
}

function _internalTriggerNetworkStatusChange(isConnected, networkType) {
    const data = { isConnected: isConnected, networkType: networkType };
    for (let i = 0; i < _listeners.length; i++) {
        try {
            _listeners[i](data);
        } catch (e) {
            console.error('onNetworkStatusChange listener error:', e);
        }
    }
}

export { onNetworkStatusChange, offNetworkStatusChange, _internalTriggerNetworkStatusChange };
