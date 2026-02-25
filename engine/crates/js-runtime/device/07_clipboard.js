import { op_set_clipboard_data, op_get_clipboard_data } from "ext:core/ops";
import { promisify } from "ext:host_v8_base/02_async.js";

const setClipboardData = promisify('setClipboardData', (opts) => op_set_clipboard_data(opts.data || ''));
const getClipboardData = promisify('getClipboardData', () => ({ data: op_get_clipboard_data() }));

export { setClipboardData, getClipboardData };
