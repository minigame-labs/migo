import { op_start_compass, op_stop_compass } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function startCompass(options) {
    return wrapAsync('startCompass', function () {
        op_start_compass();
    }, options);
}

function stopCompass(options) {
    return wrapAsync('stopCompass', function () {
        op_stop_compass();
    }, options);
}

export { startCompass, stopCompass };
