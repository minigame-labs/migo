// Shared helper: turn a received binary payload into an exact-length
// ArrayBuffer for delivery to game onMessage callbacks.
//
// Every binary event field (`event.data` / `event.dataBin`) arrives as a
// full-window Uint8Array backed by an external, exact-length ArrayBuffer (a
// Rust `ToJsBuffer`), so the common path returns that backing buffer BY
// IDENTITY with no copy. A partial view is sliced to its exact window, and a
// legacy numeric Array (from an un-regenerated snapshot) is converted through
// the primordial Uint8Array so new JS stays backward compatible.
//
// All metadata is read via primordials (internal slots), so a shadowed
// `.buffer` / `.byteOffset` / `.byteLength` on the value cannot redirect the
// result.

import { core, primordials } from "ext:core/mod.js";

const { isArrayBuffer, isTypedArray } = core;
const {
    Uint8Array,
    TypedArrayPrototypeGetBuffer,
    TypedArrayPrototypeGetByteOffset,
    TypedArrayPrototypeGetByteLength,
    ArrayBufferPrototypeGetByteLength,
    ArrayBufferPrototypeSlice,
} = primordials;

export function toExactArrayBuffer(value) {
    if (isTypedArray(value)) {
        const buffer = TypedArrayPrototypeGetBuffer(value);
        const offset = TypedArrayPrototypeGetByteOffset(value);
        const length = TypedArrayPrototypeGetByteLength(value);
        // Full-window view (every ToJsBuffer is): hand back the backing buffer
        // directly - no copy.
        if (offset === 0 && length === ArrayBufferPrototypeGetByteLength(buffer)) {
            return buffer;
        }
        // Partial window: exact slice of just this view's bytes.
        return ArrayBufferPrototypeSlice(buffer, offset, offset + length);
    }
    // Already a bare ArrayBuffer: identity.
    if (isArrayBuffer(value)) {
        return value;
    }
    // Legacy numeric Array (un-regenerated snapshot): copy through the
    // primordial Uint8Array to an exact-length backing buffer.
    return TypedArrayPrototypeGetBuffer(new Uint8Array(value));
}
