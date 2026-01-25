// TODO: 24_header -> 01_header

function parseHeaders(headers) {
    const parsed = {};
    for (let i = 0; i < headers.length; i++) {
        const [key, value] = headers[i];
        parsed[key] = value;
    }

    return parsed;
}

class Header {
    constructor(headers, statusCode, cookies = []) {
        this._headers = parseHeaders(headers);
        this._statusCode = statusCode;
        this._cookies = cookies;
    }

    get headers() {
        return this._headers;
    }

    get statusCode() {
        return this._statusCode;
    }

    toJSON() {
        return {
            header: this._headers,
            statusCode: this._statusCode,
            cookies: this._cookies
        };
    }
}

export { Header };