import * as amdshim from "ext:host_v8_base/01_amdshim.js"
import * as console from "ext:host_v8_console/01_console.js"
import * as event from "ext:host_v8_web/02_event.js";
import * as timers from "ext:host_v8_web/02_timers.js";
import * as abortSignal from "ext:host_v8_web/03_abort_signal.js";
import * as request from "ext:host_v8_network/26_request.js";
import * as save from "ext:host_v8_file/01_save.js";
import * as fileManager from "ext:host_v8_file/02_file_manager.js";
import * as codec from "ext:host_v8_utility/01_codec.js";
import * as image from "ext:host_v8_image/01_image.js";
import * as imageData from "ext:host_v8_image/01_image_data.js";

import { core } from "ext:core/mod.js";

const windowOrWorkerGlobalScope = {
    define: core.propNonEnumerable(amdshim.define),

    AbortController: core.propNonEnumerable(abortSignal.AbortController),
    AbortSignal: core.propNonEnumerable(abortSignal.AbortSignal),
    console: core.propNonEnumerable(console.console),
    // Timers
    setTimeout: core.propWritable(timers.setTimeout),
    clearTimeout: core.propWritable(timers.clearTimeout),
    setInterval: core.propWritable(timers.setInterval),
    clearInterval: core.propWritable(timers.clearInterval),
    setImmediate: core.propWritable(timers.setImmediate),
    // Events
    CloseEvent: core.propNonEnumerable(event.CloseEvent),
    CustomEvent: core.propNonEnumerable(event.CustomEvent),
    ErrorEvent: core.propNonEnumerable(event.ErrorEvent),
    Event: core.propNonEnumerable(event.Event),
    EventTarget: core.propNonEnumerable(event.EventTarget),
    MessageEvent: core.propNonEnumerable(event.MessageEvent),
    PromiseRejectionEvent: core.propNonEnumerable(event.PromiseRejectionEvent),
    ProgressEvent: core.propNonEnumerable(event.ProgressEvent),
    reportError: core.propWritable(event.reportError),

    // Network
    request: core.propWritable(request.request),

    // File
    saveFileToDisk: core.propNonEnumerable(save.saveFileToDisk),
    FileManager: core.propNonEnumerable(fileManager.BaseFileManager),

    // Utility
    encode: core.propNonEnumerable(codec.encode),
    decode: core.propNonEnumerable(codec.decode),

    // Image
    createImage: core.propNonEnumerable(image.createImage),
    createImageData: core.propNonEnumerable(imageData.createImageData),
};

export {
    windowOrWorkerGlobalScope
}
