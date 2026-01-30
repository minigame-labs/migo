const _onBeginListeners = [];
const _onEndListeners = [];

function onAudioInterruptionBegin(listener) {
    if (typeof listener === 'function') {
        _onBeginListeners.push(listener);
    }
}

function offAudioInterruptionBegin(listener) {
    if (typeof listener === 'function') {
        const index = _onBeginListeners.indexOf(listener);
        if (index !== -1) {
            _onBeginListeners.splice(index, 1);
        }
    } else {
        _onBeginListeners.length = 0;
    }
}

function onAudioInterruptionEnd(listener) {
    if (typeof listener === 'function') {
        _onEndListeners.push(listener);
    }
}

function offAudioInterruptionEnd(listener) {
    if (typeof listener === 'function') {
        const index = _onEndListeners.indexOf(listener);
        if (index !== -1) {
            _onEndListeners.splice(index, 1);
        }
    } else {
        _onEndListeners.length = 0;
    }
}

function _internalTriggerAudioInterruptionBegin() {
    _onBeginListeners.forEach(listener => {
        try {
            listener();
        } catch (error) {
            console.error("onAudioInterruptionBegin listener error:", error);
        }
    });
}

function _internalTriggerAudioInterruptionEnd() {
    _onEndListeners.forEach(listener => {
        try {
            listener();
        } catch (error) {
            console.error("onAudioInterruptionEnd listener error:", error);
        }
    });
}

export {
    onAudioInterruptionBegin,
    offAudioInterruptionBegin,
    onAudioInterruptionEnd,
    offAudioInterruptionEnd,
    _internalTriggerAudioInterruptionBegin,
    _internalTriggerAudioInterruptionEnd,
};
