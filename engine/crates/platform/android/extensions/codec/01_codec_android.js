/**
 * Android-specific codec override.
 *
 * GBK encoding/decoding is handled by Android's built-in java.nio.charset.Charset
 * via JNI, eliminating the need for the Rust `encoding_rs` crate on Android.
 *
 * All other encodings (utf8, ascii, latin1, base64, hex, utf16le) are delegated
 * to the base Rust ops unchanged.
 */
import { op_encode_gbk_android, op_decode_gbk_android } from "ext:core/ops";
import { op_encode_multi_formats, op_decode_multi_formats } from "ext:core/ops";

function _normalizeFormat(format) {
    return (typeof format === 'string' && format.length > 0) ? format : 'utf8';
}

function _normalizeBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (data && typeof data === 'object' && data.buffer instanceof ArrayBuffer) {
        return new Uint8Array(data.buffer, data.byteOffset || 0, data.byteLength || 0);
    }
    throw new TypeError("decode({ data }) expects Uint8Array | ArrayBuffer | ArrayBufferView");
}

function _isGbk(fmt) {
    const lower = fmt.toLowerCase().trim();
    return lower === 'gbk';
}

/**
 * Encode string to bytes. GBK uses Android native, others use Rust.
 */
function encode({ data, format = 'utf8' }) {
    if (typeof data !== 'string') {
        throw new TypeError("encode({ data }) expects string");
    }
    const fmt = _normalizeFormat(format);
    if (_isGbk(fmt)) {
        return op_encode_gbk_android(data).buffer;
    }
    return op_encode_multi_formats(data, fmt).buffer;
}

/**
 * Decode bytes to string. GBK uses Android native, others use Rust.
 */
function decode({ data, format = 'utf8' }) {
    const fmt = _normalizeFormat(format);
    const bytes = _normalizeBytes(data);
    if (_isGbk(fmt)) {
        return op_decode_gbk_android(bytes);
    }
    return op_decode_multi_formats(bytes, fmt);
}

export { encode, decode };
