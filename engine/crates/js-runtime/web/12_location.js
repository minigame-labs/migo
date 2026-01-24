
class Location {
    constructor() {
        this._href = 'minigame://mini-game-host';
        this._protocol = 'minigame:';
        this._host = 'mini-game-host';
        this._hostname = 'mini-game-host';
        this._port = '';
        this._pathname = '/';
        this._search = '';
        this._hash = '';
    }

    toString() {
        return this.href;
    }

    get href() {
        return this._protocol + '//' + this._host + this._pathname + this._search + this._hash;
    }

    set href(value) {
        this._parseURL(value);
    }

    get protocol() {
        return this._protocol;
    }

    set protocol(value) {
        this._protocol = value;
    }

    get host() {
        return this._host;
    }

    set host(value) {
        this._host = value;
        const colonIndex = value.indexOf(':');
        if (colonIndex !== -1) {
            this._hostname = value.substring(0, colonIndex);
            this._port = value.substring(colonIndex + 1);
        } else {
            this._hostname = value;
            this._port = '';
        }
    }

    get hostname() {
        return this._hostname;
    }

    set hostname(value) {
        this._hostname = value;
        this._host = this._port ? `${value}:${this._port}` : value;
    }

    get port() {
        return this._port;
    }

    set port(value) {
        this._port = value;
        this._host = value ? `${this._hostname}:${value}` : this._hostname;
    }

    get pathname() {
        return this._pathname;
    }

    set pathname(value) {
        this._pathname = value;
    }

    get search() {
        return this._search;
    }

    set search(value) {
        this._search = value;
    }

    get hash() {
        return this._hash;
    }

    set hash(value) {
        this._hash = value;
    }

    get origin() {
        return this._protocol + '//' + this._host;
    }

    _parseURL(url) {
        try {
            const protocolMatch = url.match(/^([^:]+):/);
            if (protocolMatch) {
                this._protocol = protocolMatch[1] + ':';
                url = url.substring(protocolMatch[0].length);
            }

            if (url.startsWith('//')) {
                url = url.substring(2);

                let hostEnd = url.indexOf('/');
                if (hostEnd === -1) hostEnd = url.length;

                const hostPart = url.substring(0, hostEnd);
                this.host = hostPart;

                url = url.substring(hostEnd);
            }

            const hashIndex = url.indexOf('#');
            if (hashIndex !== -1) {
                this._hash = url.substring(hashIndex);
                url = url.substring(0, hashIndex);
            } else {
                this._hash = '';
            }

            const searchIndex = url.indexOf('?');
            if (searchIndex !== -1) {
                this._search = url.substring(searchIndex);
                url = url.substring(0, searchIndex);
            } else {
                this._search = '';
            }

            this._pathname = url || '/';
        } catch (e) {
            console.error('Failed to parse URL:', url, e);
        }
    }
}

const location = new Location();
export { location };