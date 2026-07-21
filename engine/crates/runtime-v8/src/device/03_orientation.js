import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const _listeners = createListenerGroup('onDeviceOrientationChange');

function onDeviceOrientationChange(listener) {
    _listeners.on(listener);
}

function offDeviceOrientationChange(listener) {
    _listeners.off(listener);
}

function _internalTriggerDeviceOrientationChange(value) {
    _listeners.trigger({ value });
}

export {
    onDeviceOrientationChange,
    offDeviceOrientationChange,
    _internalTriggerDeviceOrientationChange,
};
