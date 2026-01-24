import { BaseFileManager } from "ext:host_v8_file/02_file_manager.js";
import { op_unzip } from "ext:core/ops";

let nextRequestId = 1;
const unzipCallbacks = new Map();

class FileManager extends BaseFileManager {
    constructor() {
        super();
    }

    static unzip({ zipFilePath, targetPath, success, fail, complete }) {
        const requestId = nextRequestId++;
        unzipCallbacks.set(requestId, { zipFilePath, targetPath, success, fail, complete });
        op_unzip(requestId, zipFilePath, targetPath);
    }
}

const getFileSystemManager = () => {
    return FileManager;
}

const _internalOnUnZipDone = (requestId) => {
    const cb = unzipCallbacks.get(requestId);
    if (cb) {
        unzipCallbacks.delete(requestId);
        cb.success && cb.success({ errMsg: "unzip:ok" });
        cb.complete && cb.complete({ errMsg: "unzip:ok" });
    }
};

export {
    getFileSystemManager,
    _internalOnUnZipDone,
};