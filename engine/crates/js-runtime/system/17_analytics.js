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
    // no-op stub per spec - returns undefined
}

function reportMonitor(name, value) {
    _warnOnce();
    // no-op stub per spec - returns undefined
}

function reportScene(sceneId, options) {
    _warnOnce();
    return wrapAsync('reportScene', function () {}, options);
}

function reportPerformance(id, value, dimensions) {
    _warnOnce();
    // no-op stub - returns undefined
}

export { reportEvent, reportMonitor, reportScene, reportPerformance };
