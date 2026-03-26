// Analytics APIs
//
// @stub All functions are no-op stubs that silently succeed.
// These require a backend analytics service to be useful.
// Connect to your analytics pipeline by replacing the no-op bodies.

import { wrapAsync } from "ext:host_v8_base/02_async.js";

let _analyticsWarned = false;
function _warnOnce() {
    if (!_analyticsWarned) {
        _analyticsWarned = true;
        console.debug(
            'Analytics API called but no analytics service is connected. ' +
            'Calls to reportEvent/reportMonitor/reportScene/reportPerformance are no-ops.'
        );
    }
}

function reportEvent(eventId, data) {
    _warnOnce();
    return wrapAsync('reportEvent', function () {}, {});
}

function reportMonitor(name, value) {
    _warnOnce();
    return wrapAsync('reportMonitor', function () {}, {});
}

function reportScene(sceneId, options) {
    _warnOnce();
    return wrapAsync('reportScene', function () {}, options);
}

function reportPerformance(id, value, dimensions) {
    _warnOnce();
    return wrapAsync('reportPerformance', function () {}, {});
}

export { reportEvent, reportMonitor, reportScene, reportPerformance };
