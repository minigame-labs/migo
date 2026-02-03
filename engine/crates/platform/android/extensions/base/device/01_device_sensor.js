import {
    op_start_device_motion, op_stop_device_motion,
    op_start_gyroscope, op_stop_gyroscope,
} from "ext:core/ops";
import { wrapWxAsync } from "ext:host_v8_base/02_wx_async.js";

function startDeviceMotionListening(options = {}) {
    const { interval = 'normal' } = options;
    return wrapWxAsync('startDeviceMotionListening',
        () => op_start_device_motion(interval), options);
}

function stopDeviceMotionListening(options = {}) {
    return wrapWxAsync('stopDeviceMotionListening',
        () => op_stop_device_motion(), options);
}

function startGyroscope(options = {}) {
    const { interval = 'normal' } = options;
    return wrapWxAsync('startGyroscope',
        () => op_start_gyroscope(interval), options);
}

function stopGyroscope(options = {}) {
    return wrapWxAsync('stopGyroscope',
        () => op_stop_gyroscope(), options);
}

export {
    startDeviceMotionListening,
    stopDeviceMotionListening,
    startGyroscope,
    stopGyroscope,
};
