class URL {
    _url = 'minigame://mini-game-host';
    constructor(url) {
        this._url = url;
    }

    toString() {
        return this._url;
    }

    get href() {
        return this._url;
    }

    set href(value) {
        this._url = value;
    }

    get hash() {
        const hashIndex = this._url.indexOf('#');
        return hashIndex !== -1 ? this._url.slice(hashIndex) : '';
    }

    get origin() {
        return 'minigame://';
    }

    get protocol() {
        return 'minigame:';
    }

    get host() {
        return 'mini-game-host';
    }

    get hostname() {
        return 'mini-game-host';
    }

    get port() {
        return '';
    }

    toString() {
        return this._url;
    }

    toJSON() {
        return this._url;
    }

    // TODO: Implement Blob URL creation and revocation
    static createObjectURL(blob) {
        throw new Error('createObjectURL is not supported in this environment');
        // if (!(blob instanceof Blob)) {
        //     throw new TypeError('Expected a Blob');
        // }
        // return new URL(`blob:${blob._rid}`);
    }

    static revokeObjectURL(url) {
        throw new Error('revokeObjectURL is not supported in this environment');
        // if (typeof url !== 'string' || !url.startsWith('blob:')) {
        //     throw new TypeError('Expected a blob URL');
        // }
        // const rid = url.slice(5); // Remove 'blob:' prefix
        // remove_resource_from_table(rid);
    }
}

export { URL }