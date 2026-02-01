import { core, primordials } from "ext:core/mod.js";
import { Header } from "ext:host_v8_network/01_header.js";
import { NetworkTask } from "ext:host_v8_network/03_task.js";
import { ReadableStream } from "ext:host_v8_web/06_stream.js";
import {
    abortedNetworkError, Response, ErrorResponse,
    nullBodyStatus, Exception,
} from "ext:host_v8_network/02_response.js";
import { op_fetch, op_fetch_send } from "ext:core/ops";

const {
    TypeError, JSONParse, TypedArrayPrototypeGetBuffer
} = primordials;

// -- Constants --

const KNOWN_METHODS = new Set(["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"]);
const NO_BODY_METHODS = new Set(["GET", "HEAD"]);

// -- RequestTask --

class RequestTask extends NetworkTask {
    constructor(terminator) {
        super(terminator);
        this._chunkReceivedListeners = [];
    }

    _onCleanup() {
        this._chunkReceivedListeners = [];
    }

    onChunkReceived(listener) {
        if (typeof listener !== 'function') {
            throw new TypeError('Listener must be a function');
        }
        if (this._aborted) {
            return;
        }
        this._chunkReceivedListeners.push(listener);
    }

    offChunkReceived(listener) {
        if (listener === undefined) {
            this._chunkReceivedListeners = [];
            return;
        }
        if (typeof listener !== 'function') {
            return;
        }
        const index = this._chunkReceivedListeners.indexOf(listener);
        if (index !== -1) {
            this._chunkReceivedListeners.splice(index, 1);
        }
    }

    _triggerChunkReceived(chunk) {
        if (this._aborted) {
            return;
        }
        for (const listener of this._chunkReceivedListeners) {
            try {
                listener(chunk);
            } catch (error) {
                console.error('Error in chunk received listener:', error);
            }
        }
    }
}

// -- Helpers --

function normalizeMethod(method = "GET") {
    const upper = method.toUpperCase();
    if (!KNOWN_METHODS.has(upper)) {
        throw new TypeError(`Unsupported HTTP method: ${method}`);
    }
    return upper;
}

/**
 * Append object/string data as query parameters for GET/HEAD requests.
 */
function appendQueryParams(url, data) {
    if (data == null) return url;

    let qs;
    if (typeof data === 'string') {
        qs = data;
    } else if (typeof data === 'object' && !(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data)) {
        const parts = [];
        const entries = Object.entries(data);
        for (let i = 0; i < entries.length; i++) {
            const [key, value] = entries[i];
            parts.push(encodeURIComponent(key) + '=' + encodeURIComponent(String(value)));
        }
        if (parts.length === 0) return url;
        qs = parts.join('&');
    } else {
        return url;
    }

    if (!qs) return url;
    const sep = url.includes('?') ? '&' : '?';
    return url + sep + qs;
}

/**
 * Build header list with smart Content-Type auto-detection.
 */
function fillHeaders(headers, method, data, hasBody) {
    const headerList = Object.entries(headers).map(([key, value]) => [key, String(value)]);

    // Check if Content-Type already provided (case-insensitive)
    let hasContentType = false;
    for (let i = 0; i < headerList.length; i++) {
        if (headerList[i][0].toLowerCase() === 'content-type') {
            hasContentType = true;
            break;
        }
    }

    if (!hasContentType && hasBody) {
        if (typeof data === 'object' && data !== null
            && !(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data)) {
            headerList.push(["Content-Type", "application/json"]);
        } else if (typeof data === 'string') {
            headerList.push(["Content-Type", "text/plain"]);
        } else {
            headerList.push(["Content-Type", "application/octet-stream"]);
        }
    }

    return headerList;
}

function chunkToU8(chunk) {
    return typeof chunk === "string" ? core.encode(chunk) : chunk;
}

function chunkToString(chunk) {
    return typeof chunk === "string" ? chunk : core.decode(chunk);
}

/**
 * Serialize request body to Uint8Array.
 */
function toBodyBuffer(body) {
    if (body == null) {
        return null;
    }
    if (body instanceof ArrayBuffer) {
        return new Uint8Array(body);
    }
    if (ArrayBuffer.isView(body)) {
        return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
    }
    if (typeof body === "string") {
        return core.encode(body);
    }
    if (typeof body === "object") {
        return core.encode(JSON.stringify(body));
    }
    throw new TypeError("Unsupported body type");
}

/**
 * Deserialize response body with JSON parse resilience.
 */
function fromBodyBuffer(buffer, dataType, responseType) {
    if (responseType === 'arraybuffer') {
        return TypedArrayPrototypeGetBuffer(chunkToU8(buffer));
    }
    if (dataType === 'json') {
        const text = chunkToString(buffer);
        try {
            return JSONParse(text);
        } catch (_) {
            // WeChat behavior: return raw string if JSON parse fails
            return text;
        }
    }
    return chunkToString(buffer);
}

// -- request() --

function request(options = {}) {
    const {
        url, data, header = {}, timeout = 60000, method = 'GET',
        dataType = 'json', responseType = 'text',
        enableHttp2 = false, enableQuic = false,
        enableCache = false, enableHttpDNS = false,
        success = () => {}, fail = () => {}, complete = () => {}
    } = options;

    // Validate URL
    if (!url || typeof url !== 'string') {
        const error = new ErrorResponse(0, new Exception(0, "request:fail invalid url", 0));
        queueMicrotask(() => { fail(error); complete(error); });
        return new RequestTask(null);
    }

    const methodNormalized = normalizeMethod(method);

    // For GET/HEAD: serialize object data as query parameters
    let finalUrl = url;
    let reqBody = null;
    if (data != null) {
        if (NO_BODY_METHODS.has(methodNormalized)) {
            finalUrl = appendQueryParams(url, data);
        } else {
            reqBody = toBodyBuffer(data);
        }
    }

    const headers = fillHeaders(header, methodNormalized, data, reqBody !== null);
    let reqRid = null;

    let requestRid, cancelHandleRid;
    try {
        const result = op_fetch(
            methodNormalized,
            finalUrl,
            headers,
            null,   // clientRid
            reqBody !== null || reqRid !== null,
            reqBody,
            reqRid,
            timeout
        );
        requestRid = result.requestRid;
        cancelHandleRid = result.cancelHandleRid;
    } catch (err) {
        const error = new ErrorResponse(0, new Exception(0, "request:fail " + err.message, 0));
        queueMicrotask(() => { fail(error); complete(error); });
        return new RequestTask(null);
    }

    const cancellation = {
        aborted: false,
        abort() {
            this.aborted = true;
            if (cancelHandleRid !== null) core.tryClose(cancelHandleRid);
        }
    };

    const requestTask = new RequestTask(cancellation);

    (async () => {
        try {
            const resp = await op_fetch_send(requestRid);
            if (cancellation.aborted) throw abortedNetworkError();

            if (resp?.error) {
                const error = new ErrorResponse(resp.status, new Exception(resp.status, resp.error, 0));
                fail(error);
                complete(error);
                return;
            }

            const respHeader = new Header(resp.headers, resp.status);
            requestTask._triggerHeadersReceived(respHeader);

            const cbResp = new Response(respHeader);

            if (nullBodyStatus(resp.status)) {
                core.close(resp.responseRid);
            } else if (methodNormalized === "HEAD" || methodNormalized === "CONNECT") {
                core.close(resp.responseRid);
            } else {
                try {
                    const rds = new ReadableStream(resp.responseRid);
                    const bodyBytes = await rds.readAll();
                    if (bodyBytes != null) {
                        cbResp.data = fromBodyBuffer(bodyBytes, dataType, responseType);
                    }
                } catch (err) {
                    const error = new ErrorResponse(500, new Exception(500, "read data failed: " + err, 0));
                    fail(error);
                    complete(error);
                    return;
                }
            }

            success(cbResp);
            complete(cbResp);
        } catch (err) {
            const error = cancellation.aborted
                ? abortedNetworkError()
                : new ErrorResponse(500, new Exception(500, err.message, 0));
            fail(error);
            complete(error);
        } finally {
            if (cancelHandleRid !== null) core.tryClose(cancelHandleRid);
        }
    })();

    return requestTask;
}

export { request };
