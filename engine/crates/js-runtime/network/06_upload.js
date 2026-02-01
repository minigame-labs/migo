import { core, primordials } from "ext:core/mod.js";
import { Header } from "ext:host_v8_network/01_header.js";
import { NetworkTask } from "ext:host_v8_network/03_task.js";
import {
    UploadResponse, UploadErrorResponse, Exception, abortedNetworkError,
} from "ext:host_v8_network/02_response.js";
import { op_fetch_upload, op_read_file } from "ext:core/ops";

const { TypeError } = primordials;

// -- UploadTask --

class UploadTask extends NetworkTask {
    constructor(terminator) {
        super(terminator);
        this._progressListeners = [];
    }

    _onCleanup() {
        this._progressListeners = [];
    }

    onProgressUpdate(listener) {
        if (typeof listener !== 'function') {
            throw new TypeError('Listener must be a function');
        }
        if (this._aborted) {
            return;
        }
        this._progressListeners.push(listener);
    }

    offProgressUpdate(listener) {
        if (listener === undefined) {
            this._progressListeners = [];
            return;
        }
        if (typeof listener !== 'function') {
            return;
        }
        const index = this._progressListeners.indexOf(listener);
        if (index !== -1) {
            this._progressListeners.splice(index, 1);
        }
    }

    _triggerProgress(progress, totalBytesSent, totalBytesExpectedToSend) {
        if (this._aborted) {
            return;
        }
        const data = { progress, totalBytesSent, totalBytesExpectedToSend };
        for (const listener of this._progressListeners) {
            try {
                listener(data);
            } catch (error) {
                console.error('Error in progress listener:', error);
            }
        }
    }
}

// -- Helpers --

function makeError(errno, msg) {
    return new UploadErrorResponse(errno, new Exception(errno, msg, 0));
}

function extractFilename(filePath) {
    const sep = filePath.lastIndexOf('/');
    return sep >= 0 ? filePath.substring(sep + 1) : filePath;
}

// -- uploadFile() --

function uploadFile(options = {}) {
    const {
        url, filePath, name, header = {}, formData = {},
        timeout = 60000,
        success = () => {}, fail = () => {}, complete = () => {}
    } = options;

    // Validate required fields
    if (!url || typeof url !== 'string') {
        const error = makeError(0, "uploadFile:fail invalid url");
        queueMicrotask(() => { fail(error); complete(error); });
        return new UploadTask(null);
    }
    if (!filePath || typeof filePath !== 'string') {
        const error = makeError(0, "uploadFile:fail invalid filePath");
        queueMicrotask(() => { fail(error); complete(error); });
        return new UploadTask(null);
    }
    if (!name || typeof name !== 'string') {
        const error = makeError(0, "uploadFile:fail invalid name");
        queueMicrotask(() => { fail(error); complete(error); });
        return new UploadTask(null);
    }

    const headers = Object.entries(header).map(([key, value]) => [key, String(value)]);
    const formEntries = Object.entries(formData).map(([key, value]) => [key, String(value)]);
    const filename = extractFilename(filePath);

    const cancellation = {
        aborted: false,
        abort() { this.aborted = true; }
    };

    const uploadTask = new UploadTask(cancellation);

    (async () => {
        try {
            // Read file content via file module (goes through VFS)
            const fileData = await op_read_file(filePath, undefined, undefined);
            if (cancellation.aborted) throw "aborted";

            const totalBytes = fileData.byteLength;
            uploadTask._triggerProgress(0, 0, totalBytes);

            // Send multipart upload via Rust op
            const result = await op_fetch_upload(
                url,
                fileData,
                name,
                filename,
                headers,
                formEntries,
                timeout,
            );

            if (cancellation.aborted) throw "aborted";

            if (result.error) {
                const error = makeError(result.statusCode || 0, result.error);
                fail(error);
                complete(error);
                return;
            }

            // Trigger headers received
            const respHeader = new Header(result.headers, result.statusCode);
            uploadTask._triggerHeadersReceived(respHeader);

            // Final progress at 100%
            uploadTask._triggerProgress(100, totalBytes, totalBytes);

            const resp = new UploadResponse(result.data, result.statusCode);
            success(resp);
            complete(resp);

        } catch (err) {
            if (cancellation.aborted || err === "aborted") {
                const error = abortedNetworkError();
                fail(error);
                complete(error);
            } else {
                const error = makeError(500, "uploadFile:fail " + (err.message || err));
                fail(error);
                complete(error);
            }
        }
    })();

    return uploadTask;
}

export { uploadFile };
