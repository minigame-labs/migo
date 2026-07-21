import {
    op_storage_get,
    op_storage_set,
    op_storage_remove,
    op_storage_clear,
    op_storage_info,
    op_storage_get_async,
    op_storage_set_async,
    op_storage_remove_async,
    op_storage_clear_async,
    op_storage_info_async,
    op_create_buffer_url,
    op_revoke_buffer_url,
} from "ext:core/ops";
import { promisify } from "ext:host_v8_base/02_async.js";

// ==================== Type-tagged serialization ====================
//
// Values are persisted as JSON with a type tag so that the original
// JavaScript type is faithfully reconstructed on read.
//
//   _t : "s" string | "n" number | "b" boolean
//        "o" object/array | "x" null | "u" undefined | "d" Date

function serialize(data) {
    if (data === undefined) return '{"_t":"u"}';
    if (data === null) return '{"_t":"x"}';
    if (data instanceof Date) return JSON.stringify({ _t: "d", _v: data.getTime() });
    switch (typeof data) {
        case "string":  return JSON.stringify({ _t: "s", _v: data });
        case "number":  return JSON.stringify({ _t: "n", _v: data });
        case "boolean": return JSON.stringify({ _t: "b", _v: data });
        default:        return JSON.stringify({ _t: "o", _v: data });
    }
}

function deserialize(raw) {
    if (!raw) return "";   // key not found returns empty string (spec-compatible)
    try {
        const obj = JSON.parse(raw);
        switch (obj._t) {
            case "s": return obj._v;
            case "n": return obj._v;
            case "b": return obj._v;
            case "o": return obj._v;
            case "x": return null;
            case "u": return undefined;
            case "d": return new Date(obj._v);
            default:  return raw;
        }
    } catch {
        return raw;
    }
}

// ==================== Sync APIs ====================

function setStorageSync(key, data) {
    op_storage_set(key, serialize(data));
}

function getStorageSync(key) {
    return deserialize(op_storage_get(key));
}

function removeStorageSync(key) {
    op_storage_remove(key);
}

function clearStorageSync() {
    op_storage_clear();
}

// Returns { keys: string[], currentSize: number, limitSize: number }.
// The op returns a JSON string with this exact shape, so no remapping is needed.
function getStorageInfoSync() {
    return JSON.parse(op_storage_info());
}

// ==================== Async APIs ====================

const setStorage = promisify("setStorage", (opts) => op_storage_set_async(opts.key, serialize(opts.data)));
const getStorage = promisify("getStorage", async (opts) => ({ data: deserialize(await op_storage_get_async(opts.key)) }));
const removeStorage = promisify("removeStorage", (opts) => op_storage_remove_async(opts.key));
const clearStorage = promisify("clearStorage", () => op_storage_clear_async());
const getStorageInfo = promisify("getStorageInfo", async () => JSON.parse(await op_storage_info_async()));

// ==================== Buffer URL ====================

function createBufferURL(buffer) {
    if (buffer instanceof ArrayBuffer) {
        return op_create_buffer_url(new Uint8Array(buffer));
    }
    if (ArrayBuffer.isView(buffer)) {
        return op_create_buffer_url(new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength));
    }
    throw new Error("createBufferURL:fail argument must be ArrayBuffer or TypedArray");
}

function revokeBufferURL(url) {
    op_revoke_buffer_url(url);
}

export {
    setStorageSync,
    setStorage,
    getStorageSync,
    getStorage,
    removeStorageSync,
    removeStorage,
    clearStorageSync,
    clearStorage,
    getStorageInfoSync,
    getStorageInfo,
    createBufferURL,
    revokeBufferURL,
};
