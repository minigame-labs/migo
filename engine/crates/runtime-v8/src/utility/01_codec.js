import { op_encode_multi_formats, op_decode_multi_formats } from "ext:core/ops";

function _normalizeFormat(format) {
    return (typeof format === 'string' && format.length > 0) ? format : 'utf8';
}

function _normalizeBytes(data) {
    // Accept: Uint8Array | ArrayBuffer | ArrayBufferView
    if (data instanceof Uint8Array) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);

    // ArrayBufferView (e.g. DataView, Uint16Array, etc.)
    if (data && typeof data === 'object' && data.buffer instanceof ArrayBuffer) {
        return new Uint8Array(data.buffer, data.byteOffset || 0, data.byteLength || 0);
    }

    throw new TypeError("decode({ data }) expects Uint8Array | ArrayBuffer | ArrayBufferView");
}

function encode({ data, format = 'utf8' }) {
    if (typeof data !== 'string') {
        throw new TypeError("encode({ data }) expects string");
    }
    const fmt = _normalizeFormat(format);
    // op returns a Rust ToJsBuffer -> Uint8Array-like; expose ArrayBuffer for callers
    return op_encode_multi_formats(data, fmt).buffer;
}

function decode({ data, format = 'utf8' }) {
    const fmt = _normalizeFormat(format);
    const bytes = _normalizeBytes(data);
    return op_decode_multi_formats(bytes, fmt);
}

export { encode, decode };
