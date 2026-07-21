import { op_start_device_motion, op_stop_device_motion } from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

const _grp = createListenerGroup('onDeviceMotionChange');

function onDeviceMotionChange(listener) { _grp.on(listener); }
function offDeviceMotionChange(listener) { _grp.off(listener); }

function _internalTriggerDeviceMotionChange(alpha, beta, gamma) {
    _grp.trigger({ alpha, beta, gamma });
}

function startDeviceMotionListening(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startDeviceMotionListening', function () {
        op_start_device_motion(interval);
    }, options);
}

function stopDeviceMotionListening(options = {}) {
    return wrapAsync('stopDeviceMotionListening', function () {
        op_stop_device_motion();
    }, options);
}

export {
    onDeviceMotionChange,
    offDeviceMotionChange,
    _internalTriggerDeviceMotionChange,
    startDeviceMotionListening,
    stopDeviceMotionListening,
};
