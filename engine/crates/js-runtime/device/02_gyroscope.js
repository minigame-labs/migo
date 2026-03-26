import { op_start_gyroscope, op_stop_gyroscope } from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

const _grp = createListenerGroup('onGyroscopeChange');

function onGyroscopeChange(listener) { _grp.on(listener); }
function offGyroscopeChange(listener) { _grp.off(listener); }

function _internalTriggerGyroscopeChange(x, y, z) {
    _grp.trigger({ x, y, z });
}

function startGyroscope(options = {}) {
    const { interval = 'normal' } = options;
    return wrapAsync('startGyroscope', function () {
        op_start_gyroscope(interval);
    }, options);
}

function stopGyroscope(options = {}) {
    return wrapAsync('stopGyroscope', function () {
        op_stop_gyroscope();
    }, options);
}

export {
    onGyroscopeChange,
    offGyroscopeChange,
    _internalTriggerGyroscopeChange,
    startGyroscope,
    stopGyroscope,
};
