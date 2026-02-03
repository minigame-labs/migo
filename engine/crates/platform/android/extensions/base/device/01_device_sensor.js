import {
    op_start_device_motion, op_stop_device_motion,
    op_start_gyroscope, op_stop_gyroscope,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function startDeviceMotionListening(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startDeviceMotionListening',
        () => op_start_device_motion(interval), options);
}

function stopDeviceMotionListening(options = {}) {
    return wrapAsync('stopDeviceMotionListening',
        () => op_stop_device_motion(), options);
}

function startGyroscope(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startGyroscope',
        () => op_start_gyroscope(interval), options);
}

function stopGyroscope(options = {}) {
    return wrapAsync('stopGyroscope',
        () => op_stop_gyroscope(), options);
}

export {
    startDeviceMotionListening,
    stopDeviceMotionListening,
    startGyroscope,
    stopGyroscope,
};
