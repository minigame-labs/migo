import { op_set_clipboard_data, op_get_clipboard_data } from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

function setClipboardData(options = {}) {
    const { data = '' } = options;
    return wrapAsync('setClipboardData', function () {
        op_set_clipboard_data(data);
    }, options);
}

function getClipboardData(options) {
    return wrapAsync('getClipboardData', function () {
        const data = op_get_clipboard_data();
        return { data: data };
    }, options);
}

export { setClipboardData, getClipboardData };
