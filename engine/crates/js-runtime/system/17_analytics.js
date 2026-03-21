// Analytics APIs
//
// @stub All functions are no-op stubs that silently succeed.
// These require a backend analytics service to be useful.
// Connect to your analytics pipeline by replacing the no-op bodies.

import { wrapAsync } from "ext:host_v8_base/02_async.js";

function reportEvent(eventId, data) {
    // no-op stub per spec - returns undefined
}

function reportMonitor(name, value) {
    // no-op stub per spec - returns undefined
}

function reportScene(sceneId, options) {
    return wrapAsync('reportScene', function () {}, options);
}

function reportPerformance(id, value, dimensions) {
    // no-op stub - returns undefined
}

export { reportEvent, reportMonitor, reportScene, reportPerformance };
