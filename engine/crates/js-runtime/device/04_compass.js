import { op_start_compass, op_stop_compass } from "ext:core/ops";
import { wrapAsync, createListenerGroup } from "ext:host_v8_base/02_async.js";

const _grp = createListenerGroup('onCompassChange');

function onCompassChange(listener) { _grp.on(listener); }
function offCompassChange(listener) { _grp.off(listener); }

function _internalTriggerCompassChange(direction, accuracy) {
    _grp.trigger({ direction: direction, accuracy: accuracy });
}

function startCompass(options = {}) {
    return wrapAsync('startCompass', function () {
        op_start_compass();
    }, options);
}

function stopCompass(options = {}) {
    return wrapAsync('stopCompass', function () {
        op_stop_compass();
    }, options);
}

export {
    onCompassChange,
    offCompassChange,
    _internalTriggerCompassChange,
    startCompass,
    stopCompass,
};
