// 00_gl_command_stream.js -- Task 4: private lazy-allocated typed GL command
// stream. Module state is NOT on globalThis / any canvas / any context object.
//
// Design: S4 (private buffer & hot-path), S5 (wire format + opcodes), S9
// (primordials). Loaded before 01_constants.js in the ESM list.

import {
    op_gl_submit_stream,
} from "ext:core/ops";

import { primordials } from "ext:core/mod.js";

// --- Captured primordials ---
// Captured at module load so game-mutable prototypes cannot interfere with
// the hot encoding path at call time.

const {
    ArrayBuffer,
    Uint32Array,
    Float32Array,
    TypedArrayPrototypeSet,
    TypedArrayPrototypeGetLength,
} = primordials;

// --- Wire-format constants ---

const MAGIC = 0x4D474C31;
const STREAM_VERSION = 1;
const MAX_STREAM_UNIFORM_WORDS = 512;

// Buffer size: 8192 u32 words = 32 KiB per backing.
const BUFFER_WORDS = 8192;

// --- Fixed opcode constants (1..58) ---

const OP_VIEWPORT = 1;
const OP_CLEAR = 2;
const OP_CLEAR_COLOR = 3;
const OP_CLEAR_DEPTH = 4;
const OP_CLEAR_STENCIL = 5;
const OP_ENABLE = 6;
const OP_DISABLE = 7;
const OP_USE_PROGRAM = 8;
const OP_BIND_BUFFER = 9;
const OP_BIND_TEXTURE = 10;
const OP_ACTIVE_TEXTURE = 11;
const OP_BIND_FRAMEBUFFER = 12;
const OP_BIND_RENDERBUFFER = 13;
const OP_BIND_VERTEX_ARRAY = 14;
const OP_BIND_SAMPLER = 15;
const OP_ENABLE_VERTEX_ATTRIB_ARRAY = 16;
const OP_DISABLE_VERTEX_ATTRIB_ARRAY = 17;
const OP_VERTEX_ATTRIB_POINTER = 18;
const OP_VERTEX_ATTRIB_DIVISOR = 19;
const OP_BLEND_FUNC = 20;
const OP_BLEND_FUNC_SEPARATE = 21;
const OP_BLEND_EQUATION = 22;
const OP_BLEND_EQUATION_SEPARATE = 23;
const OP_BLEND_COLOR = 24;
const OP_DEPTH_FUNC = 25;
const OP_DEPTH_MASK = 26;
const OP_DEPTH_RANGE = 27;
const OP_STENCIL_FUNC = 28;
const OP_STENCIL_FUNC_SEPARATE = 29;
const OP_STENCIL_OP = 30;
const OP_STENCIL_OP_SEPARATE = 31;
const OP_STENCIL_MASK = 32;
const OP_STENCIL_MASK_SEPARATE = 33;
const OP_CULL_FACE = 34;
const OP_FRONT_FACE = 35;
const OP_COLOR_MASK = 36;
const OP_SCISSOR = 37;
const OP_LINE_WIDTH = 38;
const OP_POLYGON_OFFSET = 39;
const OP_TEX_PARAMETER_I = 40;
const OP_TEX_PARAMETER_F = 41;
const OP_GENERATE_MIPMAP = 42;
const OP_PIXEL_STORE_I = 43;
const OP_HINT = 44;
const OP_SAMPLER_PARAMETER_I = 45;
const OP_SAMPLER_PARAMETER_F = 46;
const OP_DRAW_ARRAYS = 47;
const OP_DRAW_ELEMENTS = 48;
const OP_DRAW_ARRAYS_INSTANCED = 49;
const OP_DRAW_ELEMENTS_INSTANCED = 50;
const OP_BIND_BUFFER_BASE = 51;
const OP_BIND_BUFFER_RANGE = 52;
const OP_READ_BUFFER = 53;
const OP_UNIFORM1I = 54;
const OP_UNIFORM1F = 55;
const OP_UNIFORM2F = 56;
const OP_UNIFORM3F = 57;
const OP_UNIFORM4F = 58;

// --- Variable opcode constants (256..266) ---

const OP_UNIFORM1IV = 256;
const OP_UNIFORM1FV = 257;
const OP_UNIFORM2IV = 258;
const OP_UNIFORM2FV = 259;
const OP_UNIFORM3IV = 260;
const OP_UNIFORM3FV = 261;
const OP_UNIFORM4IV = 262;
const OP_UNIFORM4FV = 263;
const OP_UNIFORM_MATRIX2FV = 264;
const OP_UNIFORM_MATRIX3FV = 265;
const OP_UNIFORM_MATRIX4FV = 266;

// --- Module-private ping-pong state ---
// All null until the first successful encode (lazy allocation).
// Two backing ArrayBuffers: ping (index 0) and pong (index 1).

let _buf0 = null;
let _buf1 = null;
let _u32_0 = null; // Uint32Array overlay on _buf0
let _u32_1 = null;
let _f32_0 = null; // Float32Array overlay on _buf0
let _f32_1 = null;
let _activeIdx = 0; // which buffer is currently being written
let cursor = 2;    // write position (0=magic, 1=version, 2..=record start)

// Active overlay references (point to the current active buffer's views).
let _u32 = null;
let _f32 = null;

// --- Lazy allocation ---

function ensureBuffers() {
    if (_u32 !== null) return;
    const byteLen = BUFFER_WORDS * 4;
    _buf0 = new ArrayBuffer(byteLen);
    _buf1 = new ArrayBuffer(byteLen);
    _u32_0 = new Uint32Array(_buf0);
    _u32_1 = new Uint32Array(_buf1);
    _f32_0 = new Float32Array(_buf0);
    _f32_1 = new Float32Array(_buf1);
    _u32 = _u32_0;
    _f32 = _f32_0;
    _activeIdx = 0;
    cursor = 2;
    // Write header on both buffers so a freshly-swapped-to buffer is valid.
    _u32_0[0] = MAGIC;
    _u32_0[1] = STREAM_VERSION;
    _u32_1[0] = MAGIC;
    _u32_1[1] = STREAM_VERSION;
}

// --- Header pack ---
// low 12 bits = opcode, high 20 bits = total word_count.

function packHeader(opcode, wordCount) {
    return (wordCount << 12) | (opcode & 0xFFF);
}

// --- ensureFit: flush+swap if record won't fit ---
// For fixed-arity records; wordCount <= BUFFER_WORDS - 2 always holds for
// the fixed ops in this design (max 8 words), so after flush the buffer has
// room.

function ensureFit(wordCount) {
    if (cursor + wordCount > BUFFER_WORDS) {
        _submitAndSwap();
    }
}

// --- Internal submit and swap ---

function _submitAndSwap() {
    if (cursor <= 2) return; // nothing to submit
    const status = op_gl_submit_stream(_u32, cursor);
    if (status !== 0) {
        // Reset cursor before throwing so the buffer is clean.
        cursor = 2;
        throw new Error("op_gl_submit_stream returned error: " + status);
    }
    // Swap to the other buffer.
    if (_activeIdx === 0) {
        _activeIdx = 1;
        _u32 = _u32_1;
        _f32 = _f32_1;
        _u32_1[0] = MAGIC;
        _u32_1[1] = STREAM_VERSION;
    } else {
        _activeIdx = 0;
        _u32 = _u32_0;
        _f32 = _f32_0;
        _u32_0[0] = MAGIC;
        _u32_0[1] = STREAM_VERSION;
    }
    cursor = 2;
}

// --- Fixed-arity encoder implementations ---
// Each encoder:
//   1. Calls ensureBuffers() to lazily allocate.
//   2. Calls ensureFit(wordCount) to flush+swap if needed.
//   3. Writes header + fields directly to the overlay at cursor+k.
//   4. Advances cursor by wordCount.
//   5. Returns true.
//
// Field conventions:
//   u32/enum: written via _u32 overlay (natural u32 bits).
//   i32:      written via _u32 overlay after `value|0` (two's-complement bits).
//   f32:      written via _f32 overlay (preserves NaN/-0/+/-Infinity bits).
//   bool:     written as 0 or 1 via _u32 overlay.
//
// All layouts match S5 table: H=header, C=canvasId, U=u32, I=i32, F=f32, B=bool.

// 1 VIEWPORT: H C I I U U (6 words)
function encodeViewport(canvasId, x, y, width, height) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_VIEWPORT, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = x | 0;
    _u32[base + 3] = y | 0;
    _u32[base + 4] = width >>> 0;
    _u32[base + 5] = height >>> 0;
    cursor = base + 6;
    return true;
}

// 2 CLEAR: H C U (3 words)
function encodeClear(canvasId, mask) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_CLEAR, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mask >>> 0;
    cursor = base + 3;
    return true;
}

// 3 CLEAR_COLOR: H C F F F F (6 words)
function encodeClearColor(canvasId, r, g, b, a) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_CLEAR_COLOR, 6);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = r;
    _f32[base + 3] = g;
    _f32[base + 4] = b;
    _f32[base + 5] = a;
    cursor = base + 6;
    return true;
}

// 4 CLEAR_DEPTH: H C F (3 words)
function encodeClearDepth(canvasId, depth) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_CLEAR_DEPTH, 3);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = depth;
    cursor = base + 3;
    return true;
}

// 5 CLEAR_STENCIL: H C I (3 words)
function encodeClearStencil(canvasId, s) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_CLEAR_STENCIL, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = s | 0;
    cursor = base + 3;
    return true;
}

// 6 ENABLE: H C U (3 words)
function encodeEnable(canvasId, cap) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_ENABLE, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = cap >>> 0;
    cursor = base + 3;
    return true;
}

// 7 DISABLE: H C U (3 words)
function encodeDisable(canvasId, cap) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_DISABLE, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = cap >>> 0;
    cursor = base + 3;
    return true;
}

// 8 USE_PROGRAM: H C U (3 words)
function encodeUseProgram(canvasId, programId) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_USE_PROGRAM, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = programId >>> 0;
    cursor = base + 3;
    return true;
}

// 9 BIND_BUFFER: H C U I (4 words)
function encodeBindBuffer(canvasId, target, bufferId) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_BUFFER, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = bufferId | 0;
    cursor = base + 4;
    return true;
}

// 10 BIND_TEXTURE: H C U I (4 words)
function encodeBindTexture(canvasId, target, textureId) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_TEXTURE, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = textureId | 0;
    cursor = base + 4;
    return true;
}

// 11 ACTIVE_TEXTURE: H C U (3 words)
function encodeActiveTexture(canvasId, unit) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_ACTIVE_TEXTURE, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = unit >>> 0;
    cursor = base + 3;
    return true;
}

// 12 BIND_FRAMEBUFFER: H C U I (4 words)
function encodeBindFramebuffer(canvasId, target, fbId) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_FRAMEBUFFER, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = fbId | 0;
    cursor = base + 4;
    return true;
}

// 13 BIND_RENDERBUFFER: H C U I (4 words)
function encodeBindRenderbuffer(canvasId, target, rbId) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_RENDERBUFFER, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = rbId | 0;
    cursor = base + 4;
    return true;
}

// 14 BIND_VERTEX_ARRAY: H C U (3 words)
function encodeBindVertexArray(canvasId, vaoId) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_VERTEX_ARRAY, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = vaoId >>> 0;
    cursor = base + 3;
    return true;
}

// 15 BIND_SAMPLER: H C U U (4 words)
function encodeBindSampler(canvasId, unit, samplerId) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_SAMPLER, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = unit >>> 0;
    _u32[base + 3] = samplerId >>> 0;
    cursor = base + 4;
    return true;
}

// 16 ENABLE_VERTEX_ATTRIB_ARRAY: H C U (3 words)
function encodeEnableVertexAttribArray(canvasId, index) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_ENABLE_VERTEX_ATTRIB_ARRAY, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = index >>> 0;
    cursor = base + 3;
    return true;
}

// 17 DISABLE_VERTEX_ATTRIB_ARRAY: H C U (3 words)
function encodeDisableVertexAttribArray(canvasId, index) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_DISABLE_VERTEX_ATTRIB_ARRAY, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = index >>> 0;
    cursor = base + 3;
    return true;
}

// 18 VERTEX_ATTRIB_POINTER: H C U I U B I I (8 words)
function encodeVertexAttribPointer(canvasId, index, size, type, normalized, stride, offset) {
    ensureBuffers();
    ensureFit(8);
    const base = cursor;
    _u32[base] = packHeader(OP_VERTEX_ATTRIB_POINTER, 8);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = index >>> 0;
    _u32[base + 3] = size | 0;
    _u32[base + 4] = type >>> 0;
    _u32[base + 5] = normalized ? 1 : 0;
    _u32[base + 6] = stride | 0;
    _u32[base + 7] = offset | 0;
    cursor = base + 8;
    return true;
}

// 19 VERTEX_ATTRIB_DIVISOR: H C U U (4 words)
function encodeVertexAttribDivisor(canvasId, index, divisor) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_VERTEX_ATTRIB_DIVISOR, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = index >>> 0;
    _u32[base + 3] = divisor >>> 0;
    cursor = base + 4;
    return true;
}

// 20 BLEND_FUNC: H C U U (4 words)
function encodeBlendFunc(canvasId, sfactor, dfactor) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BLEND_FUNC, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = sfactor >>> 0;
    _u32[base + 3] = dfactor >>> 0;
    cursor = base + 4;
    return true;
}

// 21 BLEND_FUNC_SEPARATE: H C U U U U (6 words)
function encodeBlendFuncSeparate(canvasId, srcRGB, dstRGB, srcAlpha, dstAlpha) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_BLEND_FUNC_SEPARATE, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = srcRGB >>> 0;
    _u32[base + 3] = dstRGB >>> 0;
    _u32[base + 4] = srcAlpha >>> 0;
    _u32[base + 5] = dstAlpha >>> 0;
    cursor = base + 6;
    return true;
}

// 22 BLEND_EQUATION: H C U (3 words)
function encodeBlendEquation(canvasId, mode) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_BLEND_EQUATION, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    cursor = base + 3;
    return true;
}

// 23 BLEND_EQUATION_SEPARATE: H C U U (4 words)
function encodeBlendEquationSeparate(canvasId, modeRGB, modeAlpha) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_BLEND_EQUATION_SEPARATE, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = modeRGB >>> 0;
    _u32[base + 3] = modeAlpha >>> 0;
    cursor = base + 4;
    return true;
}

// 24 BLEND_COLOR: H C F F F F (6 words)
function encodeBlendColor(canvasId, r, g, b, a) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_BLEND_COLOR, 6);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = r;
    _f32[base + 3] = g;
    _f32[base + 4] = b;
    _f32[base + 5] = a;
    cursor = base + 6;
    return true;
}

// 25 DEPTH_FUNC: H C U (3 words)
function encodeDepthFunc(canvasId, func) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_DEPTH_FUNC, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = func >>> 0;
    cursor = base + 3;
    return true;
}

// 26 DEPTH_MASK: H C B (3 words)
function encodeDepthMask(canvasId, flag) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_DEPTH_MASK, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = flag ? 1 : 0;
    cursor = base + 3;
    return true;
}

// 27 DEPTH_RANGE: H C F F (4 words)
function encodeDepthRange(canvasId, near, far) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_DEPTH_RANGE, 4);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = near;
    _f32[base + 3] = far;
    cursor = base + 4;
    return true;
}

// 28 STENCIL_FUNC: H C U I U (5 words)
function encodeStencilFunc(canvasId, func, ref_, mask) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_FUNC, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = func >>> 0;
    _u32[base + 3] = ref_ | 0;
    _u32[base + 4] = mask >>> 0;
    cursor = base + 5;
    return true;
}

// 29 STENCIL_FUNC_SEPARATE: H C U U I U (6 words)
function encodeStencilFuncSeparate(canvasId, face, func, ref_, mask) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_FUNC_SEPARATE, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = face >>> 0;
    _u32[base + 3] = func >>> 0;
    _u32[base + 4] = ref_ | 0;
    _u32[base + 5] = mask >>> 0;
    cursor = base + 6;
    return true;
}

// 30 STENCIL_OP: H C U U U (5 words)
function encodeStencilOp(canvasId, fail, zfail, zpass) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_OP, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = fail >>> 0;
    _u32[base + 3] = zfail >>> 0;
    _u32[base + 4] = zpass >>> 0;
    cursor = base + 5;
    return true;
}

// 31 STENCIL_OP_SEPARATE: H C U U U U (6 words)
function encodeStencilOpSeparate(canvasId, face, fail, zfail, zpass) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_OP_SEPARATE, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = face >>> 0;
    _u32[base + 3] = fail >>> 0;
    _u32[base + 4] = zfail >>> 0;
    _u32[base + 5] = zpass >>> 0;
    cursor = base + 6;
    return true;
}

// 32 STENCIL_MASK: H C U (3 words)
function encodeStencilMask(canvasId, mask) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_MASK, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mask >>> 0;
    cursor = base + 3;
    return true;
}

// 33 STENCIL_MASK_SEPARATE: H C U U (4 words)
function encodeStencilMaskSeparate(canvasId, face, mask) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_STENCIL_MASK_SEPARATE, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = face >>> 0;
    _u32[base + 3] = mask >>> 0;
    cursor = base + 4;
    return true;
}

// 34 CULL_FACE: H C U (3 words)
function encodeCullFace(canvasId, mode) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_CULL_FACE, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    cursor = base + 3;
    return true;
}

// 35 FRONT_FACE: H C U (3 words)
function encodeFrontFace(canvasId, mode) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_FRONT_FACE, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    cursor = base + 3;
    return true;
}

// 36 COLOR_MASK: H C B B B B (6 words)
function encodeColorMask(canvasId, r, g, b, a) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_COLOR_MASK, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = r ? 1 : 0;
    _u32[base + 3] = g ? 1 : 0;
    _u32[base + 4] = b ? 1 : 0;
    _u32[base + 5] = a ? 1 : 0;
    cursor = base + 6;
    return true;
}

// 37 SCISSOR: H C I I I I (6 words)
function encodeScissor(canvasId, x, y, width, height) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_SCISSOR, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = x | 0;
    _u32[base + 3] = y | 0;
    _u32[base + 4] = width | 0;
    _u32[base + 5] = height | 0;
    cursor = base + 6;
    return true;
}

// 38 LINE_WIDTH: H C F (3 words)
function encodeLineWidth(canvasId, width) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_LINE_WIDTH, 3);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = width;
    cursor = base + 3;
    return true;
}

// 39 POLYGON_OFFSET: H C F F (4 words)
function encodePolygonOffset(canvasId, factor, units) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_POLYGON_OFFSET, 4);
    _u32[base + 1] = canvasId;
    _f32[base + 2] = factor;
    _f32[base + 3] = units;
    cursor = base + 4;
    return true;
}

// 40 TEX_PARAMETER_I: H C U U I (5 words)
function encodeTexParameteri(canvasId, target, pname, param) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_TEX_PARAMETER_I, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = pname >>> 0;
    _u32[base + 4] = param | 0;
    cursor = base + 5;
    return true;
}

// 41 TEX_PARAMETER_F: H C U U F (5 words)
function encodeTexParameterf(canvasId, target, pname, param) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_TEX_PARAMETER_F, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = pname >>> 0;
    _f32[base + 4] = param;
    cursor = base + 5;
    return true;
}

// 42 GENERATE_MIPMAP: H C U (3 words)
function encodeGenerateMipmap(canvasId, target) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_GENERATE_MIPMAP, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    cursor = base + 3;
    return true;
}

// 43 PIXEL_STORE_I: H C U I (4 words)
function encodePixelStorei(canvasId, pname, param) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_PIXEL_STORE_I, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = pname >>> 0;
    _u32[base + 3] = param | 0;
    cursor = base + 4;
    return true;
}

// 44 HINT: H C U U (4 words)
function encodeHint(canvasId, target, mode) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_HINT, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = mode >>> 0;
    cursor = base + 4;
    return true;
}

// 45 SAMPLER_PARAMETER_I: H U U I (4 words -- no canvas field)
function encodeSamplerParameteri(samplerId, pname, param) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_SAMPLER_PARAMETER_I, 4);
    _u32[base + 1] = samplerId >>> 0;
    _u32[base + 2] = pname >>> 0;
    _u32[base + 3] = param | 0;
    cursor = base + 4;
    return true;
}

// 46 SAMPLER_PARAMETER_F: H U U F (4 words -- no canvas field)
function encodeSamplerParameterf(samplerId, pname, param) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_SAMPLER_PARAMETER_F, 4);
    _u32[base + 1] = samplerId >>> 0;
    _u32[base + 2] = pname >>> 0;
    _f32[base + 3] = param;
    cursor = base + 4;
    return true;
}

// 47 DRAW_ARRAYS: H C U I I (5 words)
function encodeDrawArrays(canvasId, mode, first, count) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_DRAW_ARRAYS, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    _u32[base + 3] = first | 0;
    _u32[base + 4] = count | 0;
    cursor = base + 5;
    return true;
}

// 48 DRAW_ELEMENTS: H C U I U I (6 words)
function encodeDrawElements(canvasId, mode, count, type, offset) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_DRAW_ELEMENTS, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    _u32[base + 3] = count | 0;
    _u32[base + 4] = type >>> 0;
    _u32[base + 5] = offset | 0;
    cursor = base + 6;
    return true;
}

// 49 DRAW_ARRAYS_INSTANCED: H C U I I I (6 words)
function encodeDrawArraysInstanced(canvasId, mode, first, count, instanceCount) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_DRAW_ARRAYS_INSTANCED, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    _u32[base + 3] = first | 0;
    _u32[base + 4] = count | 0;
    _u32[base + 5] = instanceCount | 0;
    cursor = base + 6;
    return true;
}

// 50 DRAW_ELEMENTS_INSTANCED: H C U I U I I (7 words)
function encodeDrawElementsInstanced(canvasId, mode, count, type, offset, instanceCount) {
    ensureBuffers();
    ensureFit(7);
    const base = cursor;
    _u32[base] = packHeader(OP_DRAW_ELEMENTS_INSTANCED, 7);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = mode >>> 0;
    _u32[base + 3] = count | 0;
    _u32[base + 4] = type >>> 0;
    _u32[base + 5] = offset | 0;
    _u32[base + 6] = instanceCount | 0;
    cursor = base + 7;
    return true;
}

// 51 BIND_BUFFER_BASE: H C U U U (5 words)
function encodeBindBufferBase(canvasId, target, index, bufferId) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_BUFFER_BASE, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = index >>> 0;
    _u32[base + 4] = bufferId >>> 0;
    cursor = base + 5;
    return true;
}

// 52 BIND_BUFFER_RANGE: H C U U U I I (7 words)
function encodeBindBufferRange(canvasId, target, index, bufferId, offset, size) {
    ensureBuffers();
    ensureFit(7);
    const base = cursor;
    _u32[base] = packHeader(OP_BIND_BUFFER_RANGE, 7);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = target >>> 0;
    _u32[base + 3] = index >>> 0;
    _u32[base + 4] = bufferId >>> 0;
    _u32[base + 5] = offset | 0;
    _u32[base + 6] = size | 0;
    cursor = base + 7;
    return true;
}

// 53 READ_BUFFER: H C U (3 words)
function encodeReadBuffer(canvasId, src) {
    ensureBuffers();
    ensureFit(3);
    const base = cursor;
    _u32[base] = packHeader(OP_READ_BUFFER, 3);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = src >>> 0;
    cursor = base + 3;
    return true;
}

// 54 UNIFORM1I: H C I I (4 words)
function encodeUniform1i(canvasId, location, x) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_UNIFORM1I, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _u32[base + 3] = x | 0;
    cursor = base + 4;
    return true;
}

// 55 UNIFORM1F: H C I F (4 words)
function encodeUniform1f(canvasId, location, x) {
    ensureBuffers();
    ensureFit(4);
    const base = cursor;
    _u32[base] = packHeader(OP_UNIFORM1F, 4);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _f32[base + 3] = x;
    cursor = base + 4;
    return true;
}

// 56 UNIFORM2F: H C I F F (5 words)
function encodeUniform2f(canvasId, location, x, y) {
    ensureBuffers();
    ensureFit(5);
    const base = cursor;
    _u32[base] = packHeader(OP_UNIFORM2F, 5);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _f32[base + 3] = x;
    _f32[base + 4] = y;
    cursor = base + 5;
    return true;
}

// 57 UNIFORM3F: H C I F F F (6 words)
function encodeUniform3f(canvasId, location, x, y, z) {
    ensureBuffers();
    ensureFit(6);
    const base = cursor;
    _u32[base] = packHeader(OP_UNIFORM3F, 6);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _f32[base + 3] = x;
    _f32[base + 4] = y;
    _f32[base + 5] = z;
    cursor = base + 6;
    return true;
}

// 58 UNIFORM4F: H C I F F F F (7 words)
function encodeUniform4f(canvasId, location, x, y, z, w) {
    ensureBuffers();
    ensureFit(7);
    const base = cursor;
    _u32[base] = packHeader(OP_UNIFORM4F, 7);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _f32[base + 3] = x;
    _f32[base + 4] = y;
    _f32[base + 5] = z;
    _f32[base + 6] = w;
    cursor = base + 7;
    return true;
}

// --- Variable uniform vector/matrix encoders ---
// These use toInt32AsUint32/toFloat32AsUint32 semantics from 02_webgl_context.js.
// The caller is responsible for pre-converting the input (Task 5 wires this).
// Here the encoder receives a Uint32Array (already-converted payload).
// If payload_words > MAX_STREAM_UNIFORM_WORDS, returns false WITHOUT writing.
//
// Vector layout: H C location:I payload...    wc = 3 + payload_words
// Matrix layout: H C location:I transpose:B payload...  wc = 4 + payload_words

function _encodeVectorUniform(opcode, canvasId, location, payloadU32) {
    const payloadWords = TypedArrayPrototypeGetLength(payloadU32);
    if (payloadWords > MAX_STREAM_UNIFORM_WORDS) return false;
    ensureBuffers();
    const wc = 3 + payloadWords;
    ensureFit(wc);
    const base = cursor;
    _u32[base] = packHeader(opcode, wc);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    // Copy payload into overlay. TypedArrayPrototypeSet(target, source, offset)
    // where target=_u32, source=payloadU32, offset into target = base + 3.
    TypedArrayPrototypeSet(_u32, payloadU32, base + 3);
    cursor = base + wc;
    return true;
}

function _encodeMatrixUniform(opcode, canvasId, location, transpose, payloadU32) {
    const payloadWords = TypedArrayPrototypeGetLength(payloadU32);
    if (payloadWords > MAX_STREAM_UNIFORM_WORDS) return false;
    ensureBuffers();
    const wc = 4 + payloadWords;
    ensureFit(wc);
    const base = cursor;
    _u32[base] = packHeader(opcode, wc);
    _u32[base + 1] = canvasId;
    _u32[base + 2] = location | 0;
    _u32[base + 3] = transpose ? 1 : 0;
    TypedArrayPrototypeSet(_u32, payloadU32, base + 4);
    cursor = base + wc;
    return true;
}

// 256 UNIFORM1IV
function encodeUniform1iv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM1IV, canvasId, location, payloadU32);
}
// 257 UNIFORM1FV
function encodeUniform1fv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM1FV, canvasId, location, payloadU32);
}
// 258 UNIFORM2IV
function encodeUniform2iv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM2IV, canvasId, location, payloadU32);
}
// 259 UNIFORM2FV
function encodeUniform2fv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM2FV, canvasId, location, payloadU32);
}
// 260 UNIFORM3IV
function encodeUniform3iv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM3IV, canvasId, location, payloadU32);
}
// 261 UNIFORM3FV
function encodeUniform3fv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM3FV, canvasId, location, payloadU32);
}
// 262 UNIFORM4IV
function encodeUniform4iv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM4IV, canvasId, location, payloadU32);
}
// 263 UNIFORM4FV
function encodeUniform4fv(canvasId, location, payloadU32) {
    return _encodeVectorUniform(OP_UNIFORM4FV, canvasId, location, payloadU32);
}
// 264 UNIFORM_MATRIX2FV
function encodeUniformMatrix2fv(canvasId, location, transpose, payloadU32) {
    return _encodeMatrixUniform(OP_UNIFORM_MATRIX2FV, canvasId, location, transpose, payloadU32);
}
// 265 UNIFORM_MATRIX3FV
function encodeUniformMatrix3fv(canvasId, location, transpose, payloadU32) {
    return _encodeMatrixUniform(OP_UNIFORM_MATRIX3FV, canvasId, location, transpose, payloadU32);
}
// 266 UNIFORM_MATRIX4FV
function encodeUniformMatrix4fv(canvasId, location, transpose, payloadU32) {
    return _encodeMatrixUniform(OP_UNIFORM_MATRIX4FV, canvasId, location, transpose, payloadU32);
}

// --- flushGlCommandStream ---
// If unallocated OR cursor==2 (empty): allocation-free no-op.
// Else: submit the current buffer, reset, swap.
// Non-zero status from op throws an internal error.

function flushGlCommandStream() {
    if (_u32 === null || cursor === 2) return;
    _submitAndSwap();
}

// --- discardGlCommandStream ---
// Context-loss path: drop pending commands without submitting.
// Reset the active cursor to 2. No swap.

function discardGlCommandStream() {
    if (_u32 === null) return;
    cursor = 2;
    // Rewrite header so the buffer is clean for the next use.
    _u32[0] = MAGIC;
    _u32[1] = STREAM_VERSION;
}

// --- Exports ---

export {
    // Fixed-arity encoders
    encodeViewport,
    encodeClear,
    encodeClearColor,
    encodeClearDepth,
    encodeClearStencil,
    encodeEnable,
    encodeDisable,
    encodeUseProgram,
    encodeBindBuffer,
    encodeBindTexture,
    encodeActiveTexture,
    encodeBindFramebuffer,
    encodeBindRenderbuffer,
    encodeBindVertexArray,
    encodeBindSampler,
    encodeEnableVertexAttribArray,
    encodeDisableVertexAttribArray,
    encodeVertexAttribPointer,
    encodeVertexAttribDivisor,
    encodeBlendFunc,
    encodeBlendFuncSeparate,
    encodeBlendEquation,
    encodeBlendEquationSeparate,
    encodeBlendColor,
    encodeDepthFunc,
    encodeDepthMask,
    encodeDepthRange,
    encodeStencilFunc,
    encodeStencilFuncSeparate,
    encodeStencilOp,
    encodeStencilOpSeparate,
    encodeStencilMask,
    encodeStencilMaskSeparate,
    encodeCullFace,
    encodeFrontFace,
    encodeColorMask,
    encodeScissor,
    encodeLineWidth,
    encodePolygonOffset,
    encodeTexParameteri,
    encodeTexParameterf,
    encodeGenerateMipmap,
    encodePixelStorei,
    encodeHint,
    encodeSamplerParameteri,
    encodeSamplerParameterf,
    encodeDrawArrays,
    encodeDrawElements,
    encodeDrawArraysInstanced,
    encodeDrawElementsInstanced,
    encodeBindBufferBase,
    encodeBindBufferRange,
    encodeReadBuffer,
    encodeUniform1i,
    encodeUniform1f,
    encodeUniform2f,
    encodeUniform3f,
    encodeUniform4f,
    // Variable uniform encoders
    encodeUniform1iv,
    encodeUniform1fv,
    encodeUniform2iv,
    encodeUniform2fv,
    encodeUniform3iv,
    encodeUniform3fv,
    encodeUniform4iv,
    encodeUniform4fv,
    encodeUniformMatrix2fv,
    encodeUniformMatrix3fv,
    encodeUniformMatrix4fv,
    // Lifecycle
    flushGlCommandStream,
    discardGlCommandStream,
};
