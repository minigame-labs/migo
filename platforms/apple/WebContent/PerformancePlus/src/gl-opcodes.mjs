// The GL command-stream opcode table, as the WebContent producer sees it.
//
// Three implementations name these numbers: the Rust table in
// `engine/crates/frame-wire/src/gl_stream.rs`, the in-process JavaScript
// encoder in `engine/crates/runtime-v8/src/rendering/webgl/00_gl_command_stream.js`,
// and this file. `scripts/test-gl-opcode-agreement.sh` parses all three and
// requires them to agree exactly.
//
// The Rust table is the source. This file is derived from it, and the gate is
// what keeps that true -- an opcode added on one side and not the others is a
// producer emitting a record the reader will reject, on a device, with the
// frame simply not drawing.
//
// Numbers, not names, cross the process boundary: a record header is twelve
// bits of opcode and twenty of word count. So a mismatch here is not a type
// error anywhere, which is exactly why it needs a gate.

export const MAGIC = 0x4D474C31;
export const STREAM_VERSION = 1;

export const OP_VIEWPORT = 1;
export const OP_CLEAR = 2;
export const OP_CLEAR_COLOR = 3;
export const OP_CLEAR_DEPTH = 4;
export const OP_CLEAR_STENCIL = 5;
export const OP_ENABLE = 6;
export const OP_DISABLE = 7;
export const OP_USE_PROGRAM = 8;
export const OP_BIND_BUFFER = 9;
export const OP_BIND_TEXTURE = 10;
export const OP_ACTIVE_TEXTURE = 11;
export const OP_BIND_FRAMEBUFFER = 12;
export const OP_BIND_RENDERBUFFER = 13;
export const OP_BIND_VERTEX_ARRAY = 14;
export const OP_BIND_SAMPLER = 15;
export const OP_ENABLE_VERTEX_ATTRIB_ARRAY = 16;
export const OP_DISABLE_VERTEX_ATTRIB_ARRAY = 17;
export const OP_VERTEX_ATTRIB_POINTER = 18;
export const OP_VERTEX_ATTRIB_DIVISOR = 19;
export const OP_BLEND_FUNC = 20;
export const OP_BLEND_FUNC_SEPARATE = 21;
export const OP_BLEND_EQUATION = 22;
export const OP_BLEND_EQUATION_SEPARATE = 23;
export const OP_BLEND_COLOR = 24;
export const OP_DEPTH_FUNC = 25;
export const OP_DEPTH_MASK = 26;
export const OP_DEPTH_RANGE = 27;
export const OP_STENCIL_FUNC = 28;
export const OP_STENCIL_FUNC_SEPARATE = 29;
export const OP_STENCIL_OP = 30;
export const OP_STENCIL_OP_SEPARATE = 31;
export const OP_STENCIL_MASK = 32;
export const OP_STENCIL_MASK_SEPARATE = 33;
export const OP_CULL_FACE = 34;
export const OP_FRONT_FACE = 35;
export const OP_COLOR_MASK = 36;
export const OP_SCISSOR = 37;
export const OP_LINE_WIDTH = 38;
export const OP_POLYGON_OFFSET = 39;
export const OP_TEX_PARAMETER_I = 40;
export const OP_TEX_PARAMETER_F = 41;
export const OP_GENERATE_MIPMAP = 42;
export const OP_PIXEL_STORE_I = 43;
export const OP_HINT = 44;
export const OP_SAMPLER_PARAMETER_I = 45;
export const OP_SAMPLER_PARAMETER_F = 46;
export const OP_DRAW_ARRAYS = 47;
export const OP_DRAW_ELEMENTS = 48;
export const OP_DRAW_ARRAYS_INSTANCED = 49;
export const OP_DRAW_ELEMENTS_INSTANCED = 50;
export const OP_BIND_BUFFER_BASE = 51;
export const OP_BIND_BUFFER_RANGE = 52;
export const OP_READ_BUFFER = 53;
export const OP_UNIFORM1I = 54;
export const OP_UNIFORM1F = 55;
export const OP_UNIFORM2F = 56;
export const OP_UNIFORM3F = 57;
export const OP_UNIFORM4F = 58;
export const OP_UNIFORM1IV = 256;
export const OP_UNIFORM1FV = 257;
export const OP_UNIFORM2IV = 258;
export const OP_UNIFORM2FV = 259;
export const OP_UNIFORM3IV = 260;
export const OP_UNIFORM3FV = 261;
export const OP_UNIFORM4IV = 262;
export const OP_UNIFORM4FV = 263;
export const OP_UNIFORM_MATRIX2FV = 264;
export const OP_UNIFORM_MATRIX3FV = 265;
export const OP_UNIFORM_MATRIX4FV = 266;

/// Pack a record header: low twelve bits opcode, high twenty word count.
///
/// `wordCount` counts the header word itself. A fixture written from the opcode
/// name alone gets that wrong, and the structural validator catches it -- which
/// is the validator doing its job before anything else does.
export function packHeader(opcode, wordCount) {
  return ((wordCount & 0xfffff) << 12) | (opcode & 0xfff);
}

export function opcodeOf(header) {
  return header & 0xfff;
}

export function wordCountOf(header) {
  return (header >>> 12) & 0xfffff;
}
