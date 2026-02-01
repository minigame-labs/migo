import * as amdshim from "ext:host_v8_base/01_amdshim.js"
import * as console from "ext:host_v8_console/01_console.js"
import * as timers from "ext:host_v8_web/02_timers.js";
import * as request from "ext:host_v8_network/04_request.js";
import * as download from "ext:host_v8_network/05_download.js";
import * as upload from "ext:host_v8_network/06_upload.js";
import * as save from "ext:host_v8_file/01_save.js";
import * as fileManager from "ext:host_v8_file/02_file_manager.js";
import * as codec from "ext:host_v8_utility/01_codec.js";
import * as image from "ext:host_v8_image/01_image.js";
import * as imageData from "ext:host_v8_image/01_image_data.js";

import { core } from "ext:core/mod.js";

const windowOrWorkerGlobalScope = {
    define: core.propNonEnumerable(amdshim.define),

    console: core.propNonEnumerable(console.console),
    // Timers
    setTimeout: core.propWritable(timers.setTimeout),
    clearTimeout: core.propWritable(timers.clearTimeout),
    setInterval: core.propWritable(timers.setInterval),
    clearInterval: core.propWritable(timers.clearInterval),
    setImmediate: core.propWritable(timers.setImmediate),

    // Network
    request: core.propWritable(request.request),
    downloadFile: core.propWritable(download.downloadFile),
    uploadFile: core.propWritable(upload.uploadFile),

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
