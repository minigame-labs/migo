function parseHeaders(headers) {
    const parsed = {};
    for (let i = 0; i < headers.length; i++) {
        const [key, value] = headers[i];
        parsed[key] = value;
    }

    return parsed;
}

function nullBodyStatus(status) {
    return status === 101 || status === 204 || status === 205 || status === 304;
}


class Exception {
    constructor(errno, errMsg, retryCount = 0) {
        this._retryCount = retryCount;
        this._reasons = [
            {
                errMsg,
                errno
            }
        ];
    }

    appendReason(errno, errMsg) {
        this._reasons.push({
            errno,
            errMsg
        });
    }

    toJSON() {
        return {
            retryCount: this._retryCount,
            reasons: this._reasons
        };
    }
}

const successMsg = "request:ok";
const failMsg = "request:fail";
class ResponseBase {
    constructor(useHttpDNS, errMsg, exception) {
        this._useHttpDNS = useHttpDNS;
        this._errMsg = errMsg;
        this._exception = exception;
    }

    get useHttpDNS() {
        return this._useHttpDNS;
    }

    get exception() {
        return this._exception;
    }

    toJSON() {
        return {
            useHttpDNS: this._useHttpDNS,
            errMsg: this._errMsg,
            exception: this._exception?.toJSON()
        };
    }
}

class Response extends ResponseBase {
    constructor(header, useHttpDNS = false) {
        super(useHttpDNS, successMsg, null);
        this._header = header;
        this._data = null;
    }

    set data(value) {
        this._data = value;
    }

    get data() {
        return this._data;
    }

    get statusCode() {
        return this._header?.statusCode;
    }


    toJSON() {
        return {
            ...super.toJSON(),
            ...this._header?.toJSON(),
            data: this._data,
        };
    }
}

class ErrorResponse extends ResponseBase {
    constructor(errno, exception) {
        super(false, failMsg, exception);
        this._errno = errno;
    }

    toJSON() {
        return {
            ...super.toJSON(),
            errno: this._errno,
        };
    }
}

function abortedNetworkError(retryCount = 0) {
    return new ErrorResponse(-1, new Exception(-1, "Request Aborted", retryCount));
}

function timedOutNetworkError(retryCount = 0) {
    return new ErrorResponse(408, new Exception(408, "Request Timed Out", retryCount));
}

export { Response, ResponseBase, ErrorResponse, abortedNetworkError, timedOutNetworkError, parseHeaders, nullBodyStatus, Exception };