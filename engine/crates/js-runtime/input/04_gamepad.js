// The W3C Gamepad API.
//
// wx has no gamepad API, so the reference here is the Web platform Migo
// replaces: content that runs in a WebView calls navigator.getGamepads() and
// listens for gamepadconnected. Inventing a Migo-shaped API instead would make
// existing HTML5 games not work for no reason.
//
// The API is POLLED, not evented: content calls getGamepads() every frame and
// reads whatever is current. So state lives here and the host pushes updates
// into it, rather than each sample being dispatched to a listener that most
// content would not have.

const _gamepads = [];
const _connectedListeners = [];
const _disconnectedListeners = [];

// The Web API's slot semantics: an unplugged pad leaves a hole rather than
// shifting the ones after it, because content holds on to an index.
function getGamepads() {
    return _gamepads.slice();
}

function _makeGamepad(index, id, mapping, axisCount, buttonCount) {
    const axes = new Array(axisCount);
    for (let i = 0; i < axisCount; i++) axes[i] = 0;
    const buttons = new Array(buttonCount);
    for (let i = 0; i < buttonCount; i++) {
        buttons[i] = { pressed: false, touched: false, value: 0 };
    }
    return {
        id: id,
        index: index,
        connected: true,
        mapping: mapping,
        timestamp: 0,
        axes: axes,
        buttons: buttons,
    };
}

// ==================== Events ====================

function _addGamepadListener(group, listener) {
    if (typeof listener === 'function' && group.indexOf(listener) === -1) {
        group.push(listener);
    }
}

function _removeGamepadListener(group, listener) {
    const at = group.indexOf(listener);
    if (at !== -1) group.splice(at, 1);
}

function _fireGamepadEvent(group, type, gamepad) {
    const event = { type: type, gamepad: gamepad };
    for (let i = 0; i < group.length; i++) {
        // One listener throwing must not stop the others, and must not leave
        // the pad half-registered: content commonly adds a listener per system.
        try {
            group[i](event);
        } catch (e) {
            console.error('[gamepad] ' + type + ' listener threw: ' + e);
        }
    }
}

function onGamepadConnected(listener) {
    _addGamepadListener(_connectedListeners, listener);
}

function offGamepadConnected(listener) {
    _removeGamepadListener(_connectedListeners, listener);
}

function onGamepadDisconnected(listener) {
    _addGamepadListener(_disconnectedListeners, listener);
}

function offGamepadDisconnected(listener) {
    _removeGamepadListener(_disconnectedListeners, listener);
}

// ==================== Internal triggers (called from native) ====================

function _internalTriggerGamepadConnected(index, id, mapping, axisCount, buttonCount) {
    const gamepad = _makeGamepad(index, id, mapping, axisCount, buttonCount);
    _gamepads[index] = gamepad;
    _fireGamepadEvent(_connectedListeners, 'gamepadconnected', gamepad);
}

function _internalTriggerGamepadDisconnected(index) {
    const gamepad = _gamepads[index];
    if (!gamepad) return;
    gamepad.connected = false;
    // Emptied rather than removed: content that stored the index must see a
    // hole, not the pad that happens to be plugged in next.
    _gamepads[index] = null;
    _fireGamepadEvent(_disconnectedListeners, 'gamepaddisconnected', gamepad);
}

// One flat array per sample, laid out as
//   [axisCount, buttonCount, ...axes, ...(pressed, touched, value) per button]
// so a frame's worth of state crosses the boundary as a single value rather
// than as several nested ones.
function _internalTriggerGamepadState(index, timestampMs, packed) {
    const gamepad = _gamepads[index];
    // A state update for a slot nothing is connected to is not an error: a pad
    // can be unplugged between the host sampling it and this running.
    if (!gamepad) return;

    const axisCount = packed[0];
    const buttonCount = packed[1];
    let at = 2;
    for (let i = 0; i < axisCount; i++) {
        gamepad.axes[i] = packed[at++];
    }
    for (let i = 0; i < buttonCount; i++) {
        const button = gamepad.buttons[i];
        if (button) {
            button.pressed = packed[at] !== 0;
            button.touched = packed[at + 1] !== 0;
            button.value = packed[at + 2];
        }
        at += 3;
    }
    gamepad.timestamp = timestampMs;
}

export {
    getGamepads,
    onGamepadConnected,
    offGamepadConnected,
    onGamepadDisconnected,
    offGamepadDisconnected,
    _internalTriggerGamepadConnected,
    _internalTriggerGamepadDisconnected,
    _internalTriggerGamepadState,
};
