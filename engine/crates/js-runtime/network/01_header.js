function parseHeaders(rawHeaders) {
    const parsed = {};
    for (let i = 0; i < rawHeaders.length; i++) {
        const [key, value] = rawHeaders[i];
        parsed[key] = value;
    }
    return parsed;
}

function extractCookies(rawHeaders) {
    const cookies = [];
    for (let i = 0; i < rawHeaders.length; i++) {
        const [key, value] = rawHeaders[i];
        if (key.toLowerCase() === 'set-cookie') {
            cookies.push(value);
        }
    }
    return cookies;
}

class Header {
    constructor(rawHeaders, statusCode) {
        this._headers = parseHeaders(rawHeaders);
        this._statusCode = statusCode;
        this._cookies = extractCookies(rawHeaders);
    }

    get headers() {
        return this._headers;
    }

    get statusCode() {
        return this._statusCode;
    }

    get cookies() {
        return this._cookies;
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
