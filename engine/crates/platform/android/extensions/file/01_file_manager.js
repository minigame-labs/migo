import { BaseFileManager } from "ext:host_v8_file/02_file_manager.js";
import { op_unzip_android } from "ext:core/ops";

class FileManager extends BaseFileManager {
    constructor() {
        super();
    }

    /**
     * Extract a zip file to target directory.
     * Overrides base implementation to use Android's native java.util.zip via JNI,
     * avoiding the Rust `zip` crate dependency entirely on Android.
     *
     * @param {Object} options
     * @param {string} options.zipFilePath - Path to the zip file
     * @param {string} options.targetPath - Destination directory
     * @param {Function} [options.success] - Success callback
     * @param {Function} [options.fail] - Failure callback  
     * @param {Function} [options.complete] - Completion callback (always called)
     */
    static unzip({ zipFilePath, targetPath, success, fail, complete }) {
        op_unzip_android(zipFilePath, targetPath)
            .then(() => {
                const result = { errMsg: "unzip:ok" };
                success && success(result);
                complete && complete(result);
            })
            .catch((err) => {
                const result = { errMsg: `unzip:fail ${err.message || err}` };
                fail && fail(result);
                complete && complete(result);
            });
    }
}

const getFileSystemManager = () => {
    return FileManager;
}

export { getFileSystemManager };