import { op_open_app_authorize_setting } from "ext:core/ops";
import { createDeferredApi } from "ext:host_v8_base/02_async.js";

// Correlated by request id like every other deferred API. This settled by
// `shift()` on an array, so two calls in flight -- or one left pending across a
// runtime restart -- answered each other. Timeout disabled: the user is in the
// system settings app and takes as long as they take.
const _authSettingApi = createDeferredApi('openAppAuthorizeSetting', 0);

function openAppAuthorizeSetting(options) {
    return _authSettingApi.invoke(options || {}, function (_o, requestId) {
        op_open_app_authorize_setting(requestId);
    });
}

// The platform reports a code over JNI rather than JSON; a negative one is the
// failure, and `error` is what makes the shared settler reject.
function _internalOnOpenAppAuthorizeSettingFinished(requestId, code) {
    // Omitted when non-positive: an integer parameter cannot be absent the way
    // a JSON key can, and a `requestId` of 0 in the result would be read as
    // present-and-invalid and discarded rather than falling back.
    var result = code >= 0 ? {} : { error: 'openAppAuthorizeSetting:fail' };
    if (requestId > 0) result.requestId = requestId;
    _authSettingApi.settleParsed(result);
}

export { openAppAuthorizeSetting, _internalOnOpenAppAuthorizeSettingFinished };
