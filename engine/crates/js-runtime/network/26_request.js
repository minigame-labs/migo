import { core, primordials } from "ext:core/mod.js";
import { Header } from "ext:host_v8_network/24_header.js";
import { RequestTask } from "ext:host_v8_network/25_request_task.js";
import { add, AbortController } from "ext:host_v8_web/03_abort_signal.js";
import { ReadableStream } from "ext:host_v8_web/06_stream.js";
import { abortedNetworkError, Response, ErrorResponse, nullBodyStatus, Exception } from "ext:host_v8_network/25_response.js";
import { op_fetch, op_fetch_send } from "ext:core/ops";

const {
    TypeError, JSONParse, TypedArrayPrototypeGetBuffer
} = primordials;

const KNOWN_METHODS = new Set(["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"]);

function fillHeaders(headers = {}) {
    // Ensure Content-Type header exists and default to "application/json"
    const headerList = Object.entries(headers).map(([key, value]) => [key, String(value)]);
    if (!headers['Content-Type'] && !headers['content-type']) {
        headerList.push(["Content-Type", "application/json"]);
    }
    return headerList;
}

function normalizeMethod(method = "GET") {
    if (!KNOWN_METHODS.has(method.toUpperCase())) {
        throw new TypeError(`Unsupported HTTP method: ${method}`);
    }
    return method.toUpperCase();
}

function chunkToU8(chunk) {
    return typeof chunk === "string" ? core.encode(chunk) : chunk;
}

function chunkToString(chunk) {
    return typeof chunk === "string" ? chunk : core.decode(chunk);
}

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

function fromBodyBuffer(buffer, dataType, responseType) {
    if (responseType === 'arraybuffer') {
        return TypedArrayPrototypeGetBuffer(chunkToU8(buffer));
    }
    if (dataType === 'json') {
        return JSONParse(chunkToString(buffer));
    }
    return chunkToString(buffer);
}

// TODO: support http2 and http3, enableCache, onchunk
function request(options = {}) {
    const {
        url, data, header = {}, timeout = 60000, method = 'GET', dataType = 'json', responseType = 'text',
        enableHttp2 = false, enableQuic = false, enableCache = false, success = () => { }, fail = () => { }, complete = () => { }
    } = options;

    const methodNormalized = normalizeMethod(method);
    const headers = fillHeaders(header);

    let reqBody = toBodyBuffer(data);
    let reqRid = null;

    const { requestRid, cancelHandleRid } = op_fetch(
        methodNormalized,
        url,
        headers,
        null,   // clientRid
        reqBody !== null || reqRid !== null,
        reqBody,
        reqRid,
        timeout
    );

    const terminator = new AbortController();
    terminator.signal[add](async () => {
        if (cancelHandleRid !== null) core.tryClose(cancelHandleRid);
    });

    const requestTask = new RequestTask(terminator);

    (async () => {
        try {
            let resp = await op_fetch_send(requestRid);
            if (terminator.signal.aborted) throw abortedNetworkError();

            if (resp?.error) {
                const error = new ErrorResponse(resp.status, new Exception(resp.status, resp.error, 0));
                fail(error);
                complete(error);
                return;
            }

            const header = new Header(resp.headers, resp.status);
            // Trigger onheader received first
            requestTask._triggerHeadersReceived(header);

            const cbResp = new Response(header);
            if (nullBodyStatus(resp.status)) {
                core.close(resp.responseRid);
            } else if (methodNormalized === "HEAD" || methodNormalized === "CONNECT") {
                core.close(resp.responseRid);
            } else {
                try {
                    const rds = new ReadableStream(resp.responseRid);
                    terminator.signal[add](async () => {
                        rds.cancel();
                    });
                    cbResp.data = fromBodyBuffer(await rds.readAll(), dataType, responseType);
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
            const error = terminator.signal.aborted ? abortedNetworkError() : new ErrorResponse(500, new Exception(500, err.message, 0));
            fail(error);
            complete(error);
        } finally {
            if (cancelHandleRid !== null) core.tryClose(cancelHandleRid);
        }
    })();

    return requestTask;
}

export { request };
