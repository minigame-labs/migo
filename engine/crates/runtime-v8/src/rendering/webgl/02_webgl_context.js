import {
    op_viewport,
    op_clear,
    op_clear_color,
    op_gl_flush,
    op_gl_is_context_lost,
    op_gl_lose_context,
    op_create_program,
    op_use_program,
    op_link_program,
    op_get_program_parameter,
    op_get_program_info_log,
    op_delete_program,
    op_create_shader,
    op_shader_source,
    op_compile_shader,
    op_attach_shader,
    op_get_shader_parameter,
    op_get_shader_info_log,
    op_delete_shader,
    op_draw_arrays,
    op_draw_elements,
    op_get_attrib_location,
    op_bind_attrib_location,
    op_get_active_attrib,
    op_get_active_uniform,
    op_enable_vertex_attrib_array,
    op_vertex_attrib_pointer,
    op_create_buffer,
    op_bind_buffer,
    op_buffer_data,
    op_get_uniform_location,
    op_uniform3f,
    op_uniform_matrix_3fv,
    op_alloc_gl_resource_id,
    op_webgl_get_error,
    op_webgl_get_context_attributes,
    op_webgl_record_attributes,
    op_enable,
    op_disable,
    op_is_enabled,
    op_get_parameter,
    op_create_texture,
    op_delete_texture,
    op_bind_texture,
    op_active_texture,
    op_tex_image_2d,
    op_tex_image_2d_from_image,
    op_tex_image_2d_from_snapshot,
    op_tex_image_2d_from_canvas2d,
    op_tex_image_2d_from_text_cache,
    op_tex_sub_image_2d_from_snapshot,
    op_tex_sub_image_2d_from_canvas2d,
    op_tex_sub_image_2d,
    op_tex_sub_image_2d_from_image,
    op_tex_parameteri,
    op_tex_parameterf,
    op_generate_mipmap,
    op_pixel_storei,
    op_compressed_tex_image_2d,
    op_compressed_tex_sub_image_2d,
    op_buffer_sub_data,
    op_disable_vertex_attrib_array,
    op_clear_depth,
    op_clear_stencil,
    op_blend_func,
    op_blend_func_separate,
    op_blend_equation,
    op_blend_equation_separate,
    op_blend_color,
    op_depth_func,
    op_depth_mask,
    op_depth_range,
    op_stencil_func,
    op_stencil_func_separate,
    op_stencil_op,
    op_stencil_op_separate,
    op_stencil_mask,
    op_stencil_mask_separate,
    op_cull_face,
    op_front_face,
    op_color_mask,
    op_scissor,
    op_line_width,
    op_polygon_offset,
    op_uniform1i,
    op_uniform1f,
    op_uniform2f,
    op_uniform4f,
    op_uniform1iv,
    op_uniform1fv,
    op_uniform2iv,
    op_uniform2fv,
    op_uniform3iv,
    op_uniform3fv,
    op_uniform4iv,
    op_uniform4fv,
    op_uniform_matrix_2fv,
    op_uniform_matrix_4fv,
    op_create_framebuffer,
    op_delete_framebuffer,
    op_bind_framebuffer,
    op_framebuffer_texture_2d,
    op_framebuffer_renderbuffer,
    op_check_framebuffer_status,
    op_create_renderbuffer,
    op_delete_renderbuffer,
    op_delete_buffer,
    op_bind_renderbuffer,
    op_renderbuffer_storage,
    op_read_pixels,
    op_hint,

    // WebGL 2.0 additions
    op_create_vertex_array,
    op_delete_vertex_array,
    op_bind_vertex_array,
    op_vertex_attrib_divisor,
    op_draw_arrays_instanced,
    op_draw_elements_instanced,
    op_get_uniform_block_index,
    op_uniform_block_binding,
    op_bind_buffer_base,
    op_bind_buffer_range,
    op_tex_storage_2d,
    op_blit_framebuffer,
    op_invalidate_framebuffer,
    op_renderbuffer_storage_multisample,
    op_create_sampler,
    op_delete_sampler,
    op_bind_sampler,
    op_sampler_parameteri,
    op_sampler_parameterf,
    op_fence_sync,
    op_delete_sync,
    op_client_wait_sync,
    op_draw_buffers,
    op_read_buffer,
    op_alloc_gl_resource_id as op_alloc_gl_resource_id_webgl2,
    op_webgl_query_compressed_caps,
    op_create_query,
    op_delete_query,
    op_begin_query,
    op_end_query,
    op_get_query_parameter,
    op_create_transform_feedback,
    op_delete_transform_feedback,
    op_bind_transform_feedback,
    op_begin_transform_feedback,
    op_end_transform_feedback,
    op_pause_transform_feedback,
    op_resume_transform_feedback,
    op_transform_feedback_varyings,
    op_get_transform_feedback_varying,
    op_tex_image_3d,
    op_tex_sub_image_3d,
    op_tex_storage_3d,
} from "ext:core/ops";

import { core, primordials } from "ext:core/mod.js";

const { isArrayBuffer, isTypedArray, isDataView, isSharedArrayBuffer } = core;

const {
    ArrayIsArray,
    TypedArrayPrototypeGetBuffer,
    TypedArrayPrototypeGetByteLength,
    TypedArrayPrototypeGetByteOffset,
    TypedArrayPrototypeGetSymbolToStringTag,
    Uint8Array,
    Uint32Array,
    Int32Array,
    Float32Array,
    DataViewPrototypeGetBuffer,
    DataViewPrototypeGetByteLength,
    DataViewPrototypeGetByteOffset,
    ArrayBufferPrototypeGetByteLength,
    ReflectApply,
} = primordials;

import { WebglConstants } from "./01_constants.js";
import {
    flushGlCommandStream,
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
} from "./00_gl_command_stream.js";

// -- Ordered-raw op wrappers --
// Built once at module init. Each wrapper: flush pending stream, then dispatch.
// rest-args are ONLY on this raw path; the hot encode path has no rest allocations.
//
// Direct / no-submit ops (call without flush):
//   op_alloc_gl_resource_id, op_gl_is_context_lost, op_webgl_get_context_attributes,
//   op_webgl_record_attributes, op_webgl_query_compressed_caps.
//
// All others: orderedRaw(op) -> flushGlCommandStream() then ReflectApply.

function _makeOrderedRaw(op) {
    // Capture op reference at init time so game code cannot replace it.
    return function orderedRawOp(...args) {
        flushGlCommandStream();
        return ReflectApply(op, undefined, args);
    };
}

const _rawGlFlush            = _makeOrderedRaw(op_gl_flush);
const _rawClearColor         = _makeOrderedRaw(op_clear_color);
const _rawEnable             = _makeOrderedRaw(op_enable);
const _rawDisable            = _makeOrderedRaw(op_disable);
const _rawGlLoseContext      = _makeOrderedRaw(op_gl_lose_context);
const _rawCreateProgram      = _makeOrderedRaw(op_create_program);
const _rawUseProgram         = _makeOrderedRaw(op_use_program);
const _rawLinkProgram        = _makeOrderedRaw(op_link_program);
const _rawGetProgramParameter= _makeOrderedRaw(op_get_program_parameter);
const _rawGetProgramInfoLog  = _makeOrderedRaw(op_get_program_info_log);
const _rawDeleteProgram      = _makeOrderedRaw(op_delete_program);
const _rawCreateShader       = _makeOrderedRaw(op_create_shader);
const _rawShaderSource       = _makeOrderedRaw(op_shader_source);
const _rawCompileShader      = _makeOrderedRaw(op_compile_shader);
const _rawAttachShader       = _makeOrderedRaw(op_attach_shader);
const _rawGetShaderParameter = _makeOrderedRaw(op_get_shader_parameter);
const _rawGetShaderInfoLog   = _makeOrderedRaw(op_get_shader_info_log);
const _rawDeleteShader       = _makeOrderedRaw(op_delete_shader);
const _rawDrawArrays         = _makeOrderedRaw(op_draw_arrays);
const _rawDrawElements       = _makeOrderedRaw(op_draw_elements);
const _rawGetAttribLocation  = _makeOrderedRaw(op_get_attrib_location);
const _rawBindAttribLocation = _makeOrderedRaw(op_bind_attrib_location);
const _rawGetActiveAttrib    = _makeOrderedRaw(op_get_active_attrib);
const _rawGetActiveUniform   = _makeOrderedRaw(op_get_active_uniform);
const _rawEnableVertexAttribArray = _makeOrderedRaw(op_enable_vertex_attrib_array);
const _rawCreateBuffer       = _makeOrderedRaw(op_create_buffer);
const _rawBindBuffer         = _makeOrderedRaw(op_bind_buffer);
const _rawBufferData         = _makeOrderedRaw(op_buffer_data);
const _rawGetUniformLocation = _makeOrderedRaw(op_get_uniform_location);
const _rawGetParameter       = _makeOrderedRaw(op_get_parameter);
const _rawIsEnabled          = _makeOrderedRaw(op_is_enabled);
const _rawCreateTexture      = _makeOrderedRaw(op_create_texture);
const _rawDeleteTexture      = _makeOrderedRaw(op_delete_texture);
const _rawTexImage2D         = _makeOrderedRaw(op_tex_image_2d);
const _rawTexImage2DFromImage= _makeOrderedRaw(op_tex_image_2d_from_image);
const _rawTexImage2DFromSnapshot = _makeOrderedRaw(op_tex_image_2d_from_snapshot);
const _rawTexImage2DFromCanvas2d = _makeOrderedRaw(op_tex_image_2d_from_canvas2d);
const _rawTexImage2DFromTextCache= _makeOrderedRaw(op_tex_image_2d_from_text_cache);
const _rawTexSubImage2DFromSnapshot= _makeOrderedRaw(op_tex_sub_image_2d_from_snapshot);
const _rawTexSubImage2DFromCanvas2d= _makeOrderedRaw(op_tex_sub_image_2d_from_canvas2d);
const _rawTexSubImage2D      = _makeOrderedRaw(op_tex_sub_image_2d);
const _rawTexSubImage2DFromImage= _makeOrderedRaw(op_tex_sub_image_2d_from_image);
const _rawTexParameteri      = _makeOrderedRaw(op_tex_parameteri);
const _rawTexParameterf      = _makeOrderedRaw(op_tex_parameterf);
const _rawGenerateMipmap     = _makeOrderedRaw(op_generate_mipmap);
const _rawPixelStorei        = _makeOrderedRaw(op_pixel_storei);
const _rawCompressedTexImage2D = _makeOrderedRaw(op_compressed_tex_image_2d);
const _rawCompressedTexSubImage2D= _makeOrderedRaw(op_compressed_tex_sub_image_2d);
const _rawBufferSubData      = _makeOrderedRaw(op_buffer_sub_data);
const _rawDisableVertexAttribArray= _makeOrderedRaw(op_disable_vertex_attrib_array);
const _rawClearDepth         = _makeOrderedRaw(op_clear_depth);
const _rawClearStencil       = _makeOrderedRaw(op_clear_stencil);
const _rawBlendFunc          = _makeOrderedRaw(op_blend_func);
const _rawBlendFuncSeparate  = _makeOrderedRaw(op_blend_func_separate);
const _rawBlendEquation      = _makeOrderedRaw(op_blend_equation);
const _rawBlendEquationSeparate= _makeOrderedRaw(op_blend_equation_separate);
const _rawBlendColor         = _makeOrderedRaw(op_blend_color);
const _rawDepthFunc          = _makeOrderedRaw(op_depth_func);
const _rawDepthRange         = _makeOrderedRaw(op_depth_range);
const _rawStencilFunc        = _makeOrderedRaw(op_stencil_func);
const _rawStencilFuncSeparate= _makeOrderedRaw(op_stencil_func_separate);
const _rawStencilOp          = _makeOrderedRaw(op_stencil_op);
const _rawStencilOpSeparate  = _makeOrderedRaw(op_stencil_op_separate);
const _rawStencilMask        = _makeOrderedRaw(op_stencil_mask);
const _rawStencilMaskSeparate= _makeOrderedRaw(op_stencil_mask_separate);
const _rawCullFace           = _makeOrderedRaw(op_cull_face);
const _rawFrontFace          = _makeOrderedRaw(op_front_face);
const _rawScissor            = _makeOrderedRaw(op_scissor);
const _rawLineWidth          = _makeOrderedRaw(op_line_width);
const _rawPolygonOffset      = _makeOrderedRaw(op_polygon_offset);
const _rawUniform1iv         = _makeOrderedRaw(op_uniform1iv);
const _rawUniform1fv         = _makeOrderedRaw(op_uniform1fv);
const _rawUniform2iv         = _makeOrderedRaw(op_uniform2iv);
const _rawUniform2fv         = _makeOrderedRaw(op_uniform2fv);
const _rawUniform3iv         = _makeOrderedRaw(op_uniform3iv);
const _rawUniform3fv         = _makeOrderedRaw(op_uniform3fv);
const _rawUniform4iv         = _makeOrderedRaw(op_uniform4iv);
const _rawUniform4fv         = _makeOrderedRaw(op_uniform4fv);
const _rawHint               = _makeOrderedRaw(op_hint);
const _rawReadPixels         = _makeOrderedRaw(op_read_pixels);
const _rawCreateFramebuffer  = _makeOrderedRaw(op_create_framebuffer);
const _rawDeleteFramebuffer  = _makeOrderedRaw(op_delete_framebuffer);
const _rawBindFramebuffer    = _makeOrderedRaw(op_bind_framebuffer);
const _rawFramebufferTexture2D= _makeOrderedRaw(op_framebuffer_texture_2d);
const _rawFramebufferRenderbuffer= _makeOrderedRaw(op_framebuffer_renderbuffer);
const _rawCheckFramebufferStatus= _makeOrderedRaw(op_check_framebuffer_status);
const _rawCreateRenderbuffer = _makeOrderedRaw(op_create_renderbuffer);
const _rawDeleteRenderbuffer = _makeOrderedRaw(op_delete_renderbuffer);
const _rawDeleteBuffer       = _makeOrderedRaw(op_delete_buffer);
const _rawBindRenderbuffer   = _makeOrderedRaw(op_bind_renderbuffer);
const _rawRenderbufferStorage= _makeOrderedRaw(op_renderbuffer_storage);
// WebGL2 ordered raw ops
const _rawCreateVertexArray  = _makeOrderedRaw(op_create_vertex_array);
const _rawDeleteVertexArray  = _makeOrderedRaw(op_delete_vertex_array);
const _rawBindVertexArray    = _makeOrderedRaw(op_bind_vertex_array);
const _rawVertexAttribDivisor= _makeOrderedRaw(op_vertex_attrib_divisor);
const _rawDrawArraysInstanced= _makeOrderedRaw(op_draw_arrays_instanced);
const _rawDrawElementsInstanced= _makeOrderedRaw(op_draw_elements_instanced);
const _rawGetUniformBlockIndex= _makeOrderedRaw(op_get_uniform_block_index);
const _rawUniformBlockBinding= _makeOrderedRaw(op_uniform_block_binding);
const _rawBindBufferBase     = _makeOrderedRaw(op_bind_buffer_base);
const _rawBindBufferRange    = _makeOrderedRaw(op_bind_buffer_range);
const _rawTexStorage2D       = _makeOrderedRaw(op_tex_storage_2d);
const _rawBlitFramebuffer    = _makeOrderedRaw(op_blit_framebuffer);
const _rawInvalidateFramebuffer= _makeOrderedRaw(op_invalidate_framebuffer);
const _rawRenderbufferStorageMultisample= _makeOrderedRaw(op_renderbuffer_storage_multisample);
const _rawCreateSampler      = _makeOrderedRaw(op_create_sampler);
const _rawDeleteSampler      = _makeOrderedRaw(op_delete_sampler);
const _rawBindSampler        = _makeOrderedRaw(op_bind_sampler);
const _rawSamplerParameteri  = _makeOrderedRaw(op_sampler_parameteri);
const _rawSamplerParameterf  = _makeOrderedRaw(op_sampler_parameterf);
const _rawFenceSync          = _makeOrderedRaw(op_fence_sync);
const _rawDeleteSync         = _makeOrderedRaw(op_delete_sync);
const _rawClientWaitSync     = _makeOrderedRaw(op_client_wait_sync);
const _rawDrawBuffers        = _makeOrderedRaw(op_draw_buffers);
const _rawReadBuffer         = _makeOrderedRaw(op_read_buffer);
const _rawCreateQuery        = _makeOrderedRaw(op_create_query);
const _rawDeleteQuery        = _makeOrderedRaw(op_delete_query);
const _rawBeginQuery         = _makeOrderedRaw(op_begin_query);
const _rawEndQuery           = _makeOrderedRaw(op_end_query);
const _rawGetQueryParameter  = _makeOrderedRaw(op_get_query_parameter);
const _rawCreateTransformFeedback= _makeOrderedRaw(op_create_transform_feedback);
const _rawDeleteTransformFeedback= _makeOrderedRaw(op_delete_transform_feedback);
const _rawBindTransformFeedback= _makeOrderedRaw(op_bind_transform_feedback);
const _rawBeginTransformFeedback= _makeOrderedRaw(op_begin_transform_feedback);
const _rawEndTransformFeedback= _makeOrderedRaw(op_end_transform_feedback);
const _rawPauseTransformFeedback= _makeOrderedRaw(op_pause_transform_feedback);
const _rawResumeTransformFeedback= _makeOrderedRaw(op_resume_transform_feedback);
const _rawTransformFeedbackVaryings= _makeOrderedRaw(op_transform_feedback_varyings);
const _rawGetTransformFeedbackVarying= _makeOrderedRaw(op_get_transform_feedback_varying);
const _rawTexImage3D         = _makeOrderedRaw(op_tex_image_3d);
const _rawTexSubImage3D      = _makeOrderedRaw(op_tex_sub_image_3d);
const _rawTexStorage3D       = _makeOrderedRaw(op_tex_storage_3d);

const GL_CURRENT_QUERY = 0x8865;
const GL_INVALID_OPERATION = 0x0502;

function toTypedArray(input, Type) {
    if (isTypedArray(input)) {
        return new Type(
            TypedArrayPrototypeGetBuffer(input),
            TypedArrayPrototypeGetByteOffset(input),
            TypedArrayPrototypeGetByteLength(input) / Type.BYTES_PER_ELEMENT,
        );
    } else if (isDataView(input)) {
        return new Type(
            DataViewPrototypeGetBuffer(input),
            DataViewPrototypeGetByteOffset(input),
            DataViewPrototypeGetByteLength(input) / Type.BYTES_PER_ELEMENT,
        );
    } else if (isArrayBuffer(input)) {
        return new Type(
            input,
            0,
            ArrayBufferPrototypeGetByteLength(input) / Type.BYTES_PER_ELEMENT,
        );
    } else if (ArrayIsArray(input)) {
        // WebGL typed-list setters (uniform1iv/uniform4fv/... take an
        // `Int32List`/`Float32List`) accept a plain `sequence<GLint/GLfloat>`,
        // not only a TypedArray. Copy the array into the target typed array.
        // e.g. Phaser's multi-texture shader sets `uniform1iv(loc, [0,1,2,...])`.
        return new Type(input);
    }
    throw new TypeError("Invalid input: must be a TypedArray, DataView, or ArrayBuffer");
}

function toUnit8Array(input) {
    return toTypedArray(input, Uint8Array);
}

// Rust's fast `#[buffer]` path borrows the typed-array backing as a slice.
// A SharedArrayBuffer can be mutated concurrently by another isolate, which
// is not a valid Rust shared-slice contract. Copy shared views in V8 first;
// normal ArrayBuffer-backed target arrays retain the zero-copy path.
function ensureNonSharedTypedArray(view, Type) {
    return isSharedArrayBuffer(TypedArrayPrototypeGetBuffer(view))
        ? new Type(view)
        : view;
}

function toFloat32AsUint32(input) {
    let f32;
    if (isTypedArray(input)) {
        f32 = TypedArrayPrototypeGetSymbolToStringTag(input) === "Float32Array"
            ? input
            : new Float32Array(input);
    } else if (ArrayIsArray(input)) {
        // Float32List accepts numeric sequences. Constructing Uint32Array
        // directly would truncate each float to an integer before Rust
        // reinterprets the words, corrupting values such as 1.5.
        f32 = new Float32Array(input);
    } else if (isDataView(input)) {
        f32 = new Float32Array(
            DataViewPrototypeGetBuffer(input),
            DataViewPrototypeGetByteOffset(input),
            DataViewPrototypeGetByteLength(input) / Float32Array.BYTES_PER_ELEMENT,
        );
    } else if (isArrayBuffer(input)) {
        f32 = new Float32Array(
            input,
            0,
            ArrayBufferPrototypeGetByteLength(input) / Float32Array.BYTES_PER_ELEMENT,
        );
    } else {
        throw new TypeError("Invalid float list: must be a numeric sequence or buffer view");
    }
    f32 = ensureNonSharedTypedArray(f32, Float32Array);
    return new Uint32Array(
        TypedArrayPrototypeGetBuffer(f32),
        TypedArrayPrototypeGetByteOffset(f32),
        TypedArrayPrototypeGetByteLength(f32) / Uint32Array.BYTES_PER_ELEMENT,
    );
}

// Convert to an Int32 typed list, then expose the same bits to the fast
// borrowed u32 op. Rust copies those words into inline SmallVec storage.
function toInt32AsUint32(input) {
    let i32;
    if (isTypedArray(input)) {
        i32 = TypedArrayPrototypeGetSymbolToStringTag(input) === "Int32Array"
            ? input
            : new Int32Array(input);
    } else if (ArrayIsArray(input)) {
        i32 = new Int32Array(input);
    } else if (isDataView(input)) {
        i32 = new Int32Array(
            DataViewPrototypeGetBuffer(input),
            DataViewPrototypeGetByteOffset(input),
            DataViewPrototypeGetByteLength(input) / Int32Array.BYTES_PER_ELEMENT,
        );
    } else if (isArrayBuffer(input)) {
        i32 = new Int32Array(
            input,
            0,
            ArrayBufferPrototypeGetByteLength(input) / Int32Array.BYTES_PER_ELEMENT,
        );
    } else {
        throw new TypeError("Invalid integer list: must be a numeric sequence or buffer view");
    }
    i32 = ensureNonSharedTypedArray(i32, Int32Array);
    return new Uint32Array(
        TypedArrayPrototypeGetBuffer(i32),
        TypedArrayPrototypeGetByteOffset(i32),
        TypedArrayPrototypeGetByteLength(i32) / Uint32Array.BYTES_PER_ELEMENT,
    );
}

// Direct-path detection: cocos's `gl.texImage2D(target, ..., canvas)`
// pattern hands us an HTMLCanvasElement.  These have a numeric `_rid`
// (allocated by op_create_canvas) and a `getContext` method.  When we
// see one, we route to the GPU->GPU `op_tex_image_2d_from_canvas2d`
// instead of the legacy sourceToRawRgba->getImageData->readback dance.
function _migoIsHTMLCanvas(source) {
    return source
        && typeof source === "object"
        && typeof source._rid === "number"
        && typeof source.getContext === "function";
}

// Text texture cache HIT: `getImageData` returned a synthetic
// ImageData carrying `__migo_text_cache_key__` (the offscreen
// fillText was suppressed).  Route straight to the cached-texture
// copy; the render thread unpins the entry after the GPU copy.
// Returns true when it handled the upload.
function _migoTexImageFromTextCache(canvasId, target, level, internalformat, src) {
    if (!src || typeof src !== "object") return false;
    const k = src.__migo_text_cache_key__;
    if (!k) return false;
    _rawTexImage2DFromTextCache(
        canvasId,
        target,
        level,
        internalformat,
        k.text, k.fontRequest, k.fontSize, k.fontWeight,
        k.italic, k.fillColor, k.textAlign, k.textBaseline,
        k.canvasW, k.canvasH,
    );
    // Single-shot: clear the marker so a re-upload of the same
    // ImageData object doesn't double-consume the (already unpinned)
    // entry.
    src.__migo_text_cache_key__ = null;
    return true;
}

function sourceToRawRgba(source) {
    if (!source || typeof source !== "object") return null;

    const width = source.width | 0;
    const height = source.height | 0;
    if (width <= 0 || height <= 0) return null;

    if (source.data && (isTypedArray(source.data) || isDataView(source.data) || isArrayBuffer(source.data))) {
        return {
            width,
            height,
            data: toUnit8Array(source.data),
        };
    }

    if (typeof source.getContext === "function") {
        try {
            const ctx2d = source.getContext("2d");
            if (ctx2d && typeof ctx2d.getImageData === "function") {
                const imgData = ctx2d.getImageData(0, 0, width, height);
                if (imgData && imgData.data) {
                    return {
                        width,
                        height,
                        data: toUnit8Array(imgData.data),
                    };
                }
            }
        } catch (_) {
            // fall through to unsupported warning
        }
    }

    return null;
}

function _loc(location) {
    return location !== null && location !== undefined ? location.id : -1;
}

function nextResourceId() {
    const id = op_alloc_gl_resource_id();
    if (id <= 0) {
        throw new Error("Failed to allocate WebGL resource id");
    }
    return id;
}

class WebglObject {
    constructor(id) {
        this._id = id;
    }

    get id() {
        return this._id;
    }
}

class WebGLRenderingContext {
    constructor(canvas, options) {
        this._canvas = canvas;
        this._options = options || {};
        this._canvasId = canvas._rid;
        // Resource IDs are allocated from a runtime-global counter in Rust.
        // Nested Map: programId -> Map(name -> location)
        // Allows O(1) per-program invalidation via .delete(programId).
        this._attribLocationCache = new Map();
        this._uniformLocationCache = new Map();
        this._programParameterCache = new Map();
        // shaderId -> Map(pname -> value)
        this._shaderParameterCache = new Map();
        this._jsErrorQueue = [];

        // Client-side binding state. `getParameter(<X>_BINDING)` must return the
        // bound wrapper object (or null), per the WebGL spec -- engines commonly
        // save/restore bindings via `bindX(target, gl.getParameter(X_BINDING))`,
        // which requires the wrapper, not a raw GL handle. The render thread only
        // knows native GL handles, so we track the JS-side objects here.
        this._activeTextureUnit = 0x84c0; // TEXTURE0
        this._textureBindings2D = new Map(); // texture unit -> WebglObject|null
        this._textureBindingsCube = new Map(); // texture unit -> WebglObject|null
        this._arrayBufferBinding = null;
        this._elementArrayBufferBinding = null;
        this._programBinding = null;
        this._framebufferBinding = null;
        this._renderbufferBinding = null;

        // Record the negotiated attributes so `getContextAttributes()`
        // returns real values instead of bare spec defaults.  We do
        // not actually negotiate (backend is fixed RGBA8 + depth24 +
        // stencil8) so depth + stencil always exist -- both default true,
        // deviating from the WebGL 1.0 s5.2.1 stencil-defaults-false rule so
        // engine mask systems (Pixi/Cocos) do not skip stencil masking.
        const opts = this._options;
        const powerPref =
            opts.powerPreference === "high-performance"
                ? 1
                : opts.powerPreference === "low-power"
                ? 2
                : 0;
        op_webgl_record_attributes(
            this._canvasId,
            opts.alpha !== false, // default true
            opts.antialias !== false, // default true
            opts.depth !== false, // default true
            opts.stencil !== false, // default true: backend is fixed depth24+stencil8, so a stencil buffer always exists (engine mask systems check this attr)
            opts.premultipliedAlpha !== false, // default true
            opts.preserveDrawingBuffer === true, // default false
            powerPref,
            opts.failIfMajorPerformanceCaveat === true,
            opts.desynchronized === true,
            opts.xrCompatible === true,
        );
    }

    /** Invalidate all cached locations/params for a given program. O(1). */
    _invalidateProgramCaches(programId) {
        this._attribLocationCache.delete(programId);
        this._uniformLocationCache.delete(programId);
        this._programParameterCache.delete(programId);
    }

    _pushJsError(code) {
        this._jsErrorQueue.push(code >>> 0);
    }

    get canvas() {
        return this._canvas;
    }

    set drawingBufferColorSpace(value) {
        console.log(`Setting drawingBufferColorSpace to ${value}`);
        throw new Error("drawingBufferColorSpace not supported");
    }

    get drawingBufferColorSpace() {
        throw new Error("drawingBufferColorSpace not supported");
    }

    get drawingBufferWidth() {
        return this._canvas ? this._canvas.width : 0;
    }

    get drawingBufferHeight() {
        return this._canvas ? this._canvas.height : 0;
    }

    set unpackColorSpace(value) {
        console.log(`Setting unpackColorSpace to ${value}`);
        throw new Error("unpackColorSpace not supported");
    }

    get unpackColorSpace() {
        throw new Error("unpackColorSpace not supported");
    }

    viewport(x, y, width, height) {
        // Encodability: all 4 are i32/u32. Check typeof number before x|0/>>>0.
        if (typeof x === "number" && typeof y === "number" &&
            typeof width === "number" && typeof height === "number") {
            encodeViewport(this._canvasId, x | 0, y | 0, width >>> 0, height >>> 0);
            return;
        }
        // Raw fallback: flush pending stream then call original op.
        flushGlCommandStream();
        op_viewport(this._canvasId, x, y, width, height);
    }

    clearColor(r, g, b, a) {
        if (typeof r === "number" && typeof g === "number" &&
            typeof b === "number" && typeof a === "number") {
            encodeClearColor(this._canvasId, r, g, b, a);
            return;
        }
        _rawClearColor(this._canvasId, r, g, b, a);
    }

    clear(mask) {
        // u32: check number.
        if (typeof mask === "number") {
            encodeClear(this._canvasId, mask >>> 0);
            return;
        }
        flushGlCommandStream();
        op_clear(this._canvasId, mask);
    }

    // WebGL `flush()` forces queued commands to begin execution;
    // `finish()` additionally blocks until they complete.  Our backend
    // batches GL commands into the unified frame collector and dispatches
    // them to the render thread on a barrier flush, so both map to a
    // barrier flush here.  We do not expose a true GPU fence/glFinish
    // round-trip: Cocos calls finish() on resume (onShow) purely to drain
    // any commands queued before the surface was lost, which the barrier
    // flush satisfies.  Defining these is required: without them onShow
    // throws "finish is not a function" and the resume listener chain aborts.
    flush() {
        _rawGlFlush();
    }

    finish() {
        _rawGlFlush();
    }

    createProgram() {
        const id = nextResourceId();
        // op_create_program: ordered raw (not in encoded set).
        _rawCreateProgram(this._canvasId, id);
        return new WebglObject(id);
    }

    useProgram(program) {
        this._programBinding = program || null;
        const programId = program?.id ?? 0;
        // useProgram: opcode 8, H C U. programId is u32.
        if (typeof programId === "number") {
            encodeUseProgram(this._canvasId, programId >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawUseProgram(this._canvasId, program?.id);
    }

    linkProgram(program) {
        const programId = program?.id;
        _rawLinkProgram(programId);
        if (programId !== undefined) {
            // Linking can change active attrib/uniform locations and link status.
            this._invalidateProgramCaches(programId);
        }
    }

    getProgramParameter(program, pname) {
        const programId = program?.id;
        if (programId === undefined) return 0;
        let inner = this._programParameterCache.get(programId);
        if (inner) {
            const cached = inner.get(pname);
            if (cached !== undefined) {
                if (
                    pname === WebglConstants.DELETE_STATUS ||
                    pname === WebglConstants.VALIDATE_STATUS ||
                    pname === WebglConstants.LINK_STATUS
                ) {
                    return Boolean(cached);
                }
                return cached;
            }
        } else {
            inner = new Map();
            this._programParameterCache.set(programId, inner);
        }
        const param = _rawGetProgramParameter(programId, pname);
        inner.set(pname, param);
        if (
            pname === WebglConstants.DELETE_STATUS ||
            pname === WebglConstants.VALIDATE_STATUS ||
            pname === WebglConstants.LINK_STATUS
        ) {
            return Boolean(param);
        }
        return param;
    }

    getProgramInfoLog(program) {
        return _rawGetProgramInfoLog(program?.id);
    }

    deleteProgram(program) {
        const programId = program?.id;
        _rawDeleteProgram(programId);
        if (programId !== undefined) {
            this._invalidateProgramCaches(programId);
        }
    }

    createShader(type) {
        const id = nextResourceId();
        _rawCreateShader(this._canvasId, id, type);
        return new WebglObject(id);
    }

    shaderSource(shader, src) {
        return _rawShaderSource(shader?.id, src);
    }

    compileShader(shader) {
        _rawCompileShader(shader?.id);
    }

    getShaderParameter(shader, pname) {
        const shaderId = shader?.id;
        if (shaderId === undefined) return 0;
        // SHADER_TYPE is immutable after creation -- always cacheable.
        if (pname === WebglConstants.SHADER_TYPE) {
            let inner = this._shaderParameterCache.get(shaderId);
            if (inner) {
                const cached = inner.get(pname);
                if (cached !== undefined) return cached;
            } else {
                inner = new Map();
                this._shaderParameterCache.set(shaderId, inner);
            }
            const val = _rawGetShaderParameter(shaderId, pname);
            inner.set(pname, val);
            return val;
        }
        const ret = _rawGetShaderParameter(shaderId, pname);
        if (pname === WebglConstants.COMPILE_STATUS || pname === WebglConstants.DELETE_STATUS) {
            return Boolean(ret);
        }
        return ret;
    }

    attachShader(program, shader) {
        return _rawAttachShader(program?.id, shader?.id);
    }

    getShaderInfoLog(shader) {
        return _rawGetShaderInfoLog(shader?.id);
    }

    deleteShader(shader) {
        const shaderId = shader?.id;
        _rawDeleteShader(shaderId);
        if (shaderId !== undefined) {
            this._shaderParameterCache.delete(shaderId);
        }
    }

    drawArrays(mode, first, count) {
        // opcode 47: H C U I I. mode is u32, first/count are i32.
        if (typeof mode === "number" && typeof first === "number" && typeof count === "number") {
            encodeDrawArrays(this._canvasId, mode >>> 0, first | 0, count | 0);
            return;
        }
        flushGlCommandStream();
        _rawDrawArrays(this._canvasId, mode, first, count);
    }

    drawElements(mode, count, type, offset) {
        // opcode 48: H C U I U I. mode/type are u32, count/offset are i32.
        if (typeof mode === "number" && typeof count === "number" &&
            typeof type === "number" && typeof offset === "number") {
            encodeDrawElements(this._canvasId, mode >>> 0, count | 0, type >>> 0, offset | 0);
            return;
        }
        flushGlCommandStream();
        _rawDrawElements(this._canvasId, mode, count, type, offset);
    }

    bindAttribLocation(program, index, name) {
        const programId = program?.id;
        if (programId === undefined) return;
        _rawBindAttribLocation(programId, index >>> 0, name);
        // Locations only change on the next link; drop any cached lookups.
        this._attribLocationCache.delete(programId);
    }

    isContextLost() {
        // Direct, no submit: op_gl_is_context_lost is host-local.
        return op_gl_is_context_lost();
    }

    getShaderPrecisionFormat(_shaderType, precisionType) {
        // Migo is a WebGL2 / GLES3 context: highp is guaranteed in both vertex
        // and fragment shaders. Return spec-correct GLES3 values. Integer
        // precision types are HIGH_INT/MEDIUM_INT/LOW_INT (0x8DF3..0x8DF5).
        const isInt = precisionType >= 0x8df3 && precisionType <= 0x8df5;
        return isInt
            ? { rangeMin: 31, rangeMax: 30, precision: 0 }
            : { rangeMin: 127, rangeMax: 127, precision: 23 };
    }

    getAttribLocation(program, name) {
        const programId = program?.id;
        if (programId === undefined) return -1;
        let inner = this._attribLocationCache.get(programId);
        if (inner) {
            const cached = inner.get(name);
            if (cached !== undefined) return cached;
        } else {
            inner = new Map();
            this._attribLocationCache.set(programId, inner);
        }
        const location = _rawGetAttribLocation(this._canvasId, programId, name);
        inner.set(name, location);
        return location;
    }

    getActiveAttrib(program, index) {
        const programId = program?.id;
        if (programId === undefined) return null;
        const json = _rawGetActiveAttrib(this._canvasId, programId, index >>> 0);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    getActiveUniform(program, index) {
        const programId = program?.id;
        if (programId === undefined) return null;
        const json = _rawGetActiveUniform(this._canvasId, programId, index >>> 0);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    enableVertexAttribArray(index) {
        // opcode 16: H C U. index is u32.
        if (typeof index === "number") {
            encodeEnableVertexAttribArray(this._canvasId, index >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawEnableVertexAttribArray(this._canvasId, index);
    }

    vertexAttribPointer(index, size, type, normalized, stride, offset) {
        // opcode 18: H C U I U B I I.
        // index/type are u32, size/stride/offset are i32, normalized is bool.
        if (typeof index === "number" && typeof size === "number" &&
            typeof type === "number" && typeof normalized === "boolean" &&
            typeof stride === "number" && typeof offset === "number") {
            encodeVertexAttribPointer(
                this._canvasId,
                index >>> 0,
                size | 0,
                type >>> 0,
                normalized,
                stride | 0,
                offset | 0,
            );
            return;
        }
        flushGlCommandStream();
        op_vertex_attrib_pointer(
            this._canvasId,
            index,
            size,
            type,
            normalized,
            stride,
            offset,
        );
    }

    createBuffer() {
        const id = nextResourceId();
        _rawCreateBuffer(this._canvasId, id);
        return new WebglObject(id);
    }

    deleteBuffer(buffer) {
        // Per WebGL: deleting a bound buffer unbinds it from the current target.
        if (this._arrayBufferBinding === buffer) this._arrayBufferBinding = null;
        if (this._elementArrayBufferBinding === buffer) this._elementArrayBufferBinding = null;
        if (buffer && buffer.id !== undefined) _rawDeleteBuffer(buffer.id);
    }

    bindBuffer(target, buffer) {
        const buf = buffer || null;
        if (target === 0x8892) this._arrayBufferBinding = buf; // ARRAY_BUFFER
        else if (target === 0x8893) this._elementArrayBufferBinding = buf; // ELEMENT_ARRAY_BUFFER
        const bufferId = buffer?.id ?? -1;
        // opcode 9: H C U I. target is u32, bufferId is i32 (negative = unbind).
        if (typeof target === "number" && typeof bufferId === "number") {
            encodeBindBuffer(this._canvasId, target >>> 0, bufferId | 0);
            return;
        }
        flushGlCommandStream();
        _rawBindBuffer(this._canvasId, target, buffer?.id || -1);
    }

    bufferData(target, srcOrSize, usage) {
        if (typeof srcOrSize === "number") {
            const size = srcOrSize >>> 0;
            return _rawBufferData(this._canvasId, target, size, null, usage);
        } else {
            const u8 = toUnit8Array(srcOrSize);
            return _rawBufferData(this._canvasId, target, -1, u8, usage);
        }
    }

    getUniformLocation(program, name) {
        const programId = program?.id;
        if (programId === undefined) return null;
        let inner = this._uniformLocationCache.get(programId);
        if (inner) {
            const cached = inner.get(name);
            if (cached !== undefined) return cached;
        } else {
            inner = new Map();
            this._uniformLocationCache.set(programId, inner);
        }
        const id = _rawGetUniformLocation(this._canvasId, programId, name);
        if (id < 0) {
            inner.set(name, null);
            return null;
        }
        const location = new WebglObject(id);
        inner.set(name, location);
        return location;
    }

    uniform3f(location, x, y, z) {
        // opcode 57: H C I F F F. location is i32, x/y/z are f32.
        // All f32 values are encodable. Check location is a number (or null -> -1).
        const loc = _loc(location);
        encodeUniform3f(this._canvasId, loc, +x, +y, +z);
    }

    uniformMatrix3fv(location, transpose, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (typeof transpose !== "boolean" ||
            !encodeUniformMatrix3fv(this._canvasId, loc, transpose, payload)) {
            // Payload > 512 words: flush pending stream, then call raw op.
            flushGlCommandStream();
            op_uniform_matrix_3fv(this._canvasId, loc, transpose, payload);
        }
    }

    // -- Phase 1A: GL State --

    enable(cap) {
        // opcode 6: H C U.
        if (typeof cap === "number") {
            encodeEnable(this._canvasId, cap >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawEnable(this._canvasId, cap);
    }

    disable(cap) {
        // opcode 7: H C U.
        if (typeof cap === "number") {
            encodeDisable(this._canvasId, cap >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawDisable(this._canvasId, cap);
    }

    isEnabled(cap) {
        return Boolean(_rawIsEnabled(this._canvasId, cap));
    }

    getParameter(pname) {
        // Binding-state queries return the JS-side wrapper object (or null), per
        // the WebGL spec, so `bindX(target, getParameter(X_BINDING))` round-trips.
        switch (pname) {
            case 0x8069: return this._textureBindings2D.get(this._activeTextureUnit) || null; // TEXTURE_BINDING_2D
            case 0x8514: return this._textureBindingsCube.get(this._activeTextureUnit) || null; // TEXTURE_BINDING_CUBE_MAP
            case 0x8894: return this._arrayBufferBinding; // ARRAY_BUFFER_BINDING
            case 0x8895: return this._elementArrayBufferBinding; // ELEMENT_ARRAY_BUFFER_BINDING
            case 0x8b8d: return this._programBinding; // CURRENT_PROGRAM
            case 0x8ca6: return this._framebufferBinding; // FRAMEBUFFER_BINDING
            case 0x8ca7: return this._renderbufferBinding; // RENDERBUFFER_BINDING
            default: break;
        }
        const json = _rawGetParameter(this._canvasId, pname);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    getError() {
        // CRITICAL (design s8, s2): flush the stream FIRST, unconditionally,
        // so that pending stream records are decoded and their validators push
        // any errors into the host queue BEFORE we observe the error state.
        // This must happen even when _jsErrorQueue is non-empty, because a
        // later getError() call needs to find the stream-produced host errors.
        flushGlCommandStream();
        // JS-queue priority: return JS errors before host errors (two-level
        // queue semantics per WebGL 1.0 spec s5.14.3).
        if (this._jsErrorQueue.length > 0) {
            return this._jsErrorQueue.shift();
        }
        // Drain one entry from the host-side per-context WebGL error queue.
        return op_webgl_get_error(this._canvasId);
    }

    getContextAttributes() {
        // Direct, no-submit: op_webgl_get_context_attributes is host-local.
        return op_webgl_get_context_attributes(this._canvasId);
    }

    getExtension(name) {
        // Our GL backend IS GLES 3.0, so every "extension" that maps
        // to core GLES3 features we can satisfy by wiring the core
        // ops behind the OES_/ANGLE_/etc alias the WebGL 1 games
        // expect.  Cocos Creator 2.x in particular queries
        // `OES_vertex_array_object` and falls back to a per-draw
        // `vertexAttribPointer` storm when the extension is absent
        // -- exposing it here cuts the storm to one bind per
        // material at the cost of 3 one-line wrappers.
        if (name === 'OES_vertex_array_object') {
            return this._oesVertexArrayObject ||
                (this._oesVertexArrayObject = this._buildOesVertexArrayObject());
        }
        // Standard debug/robustness extension. loseContext() arms a one-shot
        // simulated GPU reset on the render thread, driving the real
        // context-loss -> recovery pipeline (webglcontextlost/restored events,
        // isContextLost()); the runtime recovers automatically on the next
        // frame, so restoreContext() is a no-op here. Lets engines (and our own
        // device tests) exercise context loss without a real driver reset.
        if (name === 'WEBGL_lose_context') {
            return this._webglLoseContext ||
                (this._webglLoseContext = {
                    loseContext: () => { _rawGlLoseContext(this._canvasId); },
                    restoreContext: () => {},
                });
        }
        // Instanced drawing: the ops are already the WebGL2 variants
        // (ES 3.0 core), so we alias the ANGLE_/EXT_ spellings to the
        // same bindings.  Cocos Creator 2.x's particle system and
        // any sprite-batcher that uses instance IDs will go from
        // N draw calls per frame to 1 -- the single largest
        // draw-call reduction available for WebGL 1 content on this
        // runtime.
        if (name === 'ANGLE_instanced_arrays' ||
            name === 'EXT_instanced_arrays' ||
            name === 'WEBGL_instanced_arrays') {
            return this._angleInstancedArrays ||
                (this._angleInstancedArrays = this._buildAngleInstancedArrays());
        }
        // Multiple render targets.  The underlying op is the WebGL 2
        // `drawBuffers`; the WebGL 1 alias just re-spells the method
        // name and enum prefix, so Cocos / three.js deferred paths
        // can detect support and light up G-buffer rendering.
        if (name === 'WEBGL_draw_buffers') {
            return this._webglDrawBuffers ||
                (this._webglDrawBuffers = this._buildWebglDrawBuffers());
        }
        // Compressed texture uploads.  The Rust backend accepts
        // ETC2/EAC unconditionally (GLES 3.0 core) and ASTC when
        // the device advertises GL_KHR_texture_compression_astc_*.
        // Exposing the extensions here lets engines pick the
        // compressed asset path instead of falling back to RGBA,
        // which can save ~16 MiB of heap per 2048^2 texture.
        if (name === 'WEBGL_compressed_texture_etc' ||
            name === 'WEBGL_compressed_texture_etc1') {
            if (!(this._compressedCaps & 1)) return null;
            return this._webglCompressedEtc ||
                (this._webglCompressedEtc = this._buildCompressedEtc());
        }
        if (name === 'WEBGL_compressed_texture_astc') {
            if (!(this._compressedCaps & 2)) return null;
            return this._webglCompressedAstc ||
                (this._webglCompressedAstc = this._buildCompressedAstc());
        }
        // 32-bit element indices are GLES 3.0 core (drawElements honors
        // UNSIGNED_INT), so expose the WebGL 1 extension alias. Without it,
        // engines (Pixi, three.js) assume 16-bit-only and cap batches at 65535
        // indices ("does not support 32 index buffer"), forcing extra draw calls
        // for large scenes. The extension object carries no methods -- its mere
        // presence signals support.
        if (name === 'OES_element_index_uint') {
            return this._oesElementIndexUint || (this._oesElementIndexUint = {});
        }
        return null;
    }

    getSupportedExtensions() {
        // Mirror the subset `getExtension` actually honours so
        // engines that probe the list before requesting (three.js,
        // pixi.js in some configurations) see the expected set.
        const list = [
            'OES_vertex_array_object',
            'ANGLE_instanced_arrays',
            'EXT_instanced_arrays',
            'WEBGL_instanced_arrays',
            'WEBGL_draw_buffers',
            'OES_element_index_uint',
            'WEBGL_lose_context',
        ];
        const caps = this._compressedCaps;
        if (caps & 1) {
            list.push('WEBGL_compressed_texture_etc');
            list.push('WEBGL_compressed_texture_etc1');
        }
        if (caps & 2) {
            list.push('WEBGL_compressed_texture_astc');
        }
        return list;
    }

    // Compressed-texture caps snapshot.  Lazily read once per
    // context; the render thread sets the caps before any JS GL
    // call completes, so caching this is safe.  Bit 0 = ETC2,
    // bit 1 = ASTC.  See `op_webgl_query_compressed_caps`.
    get _compressedCaps() {
        if (this._compressedCapsCache === undefined) {
            this._compressedCapsCache = op_webgl_query_compressed_caps() | 0;
        }
        return this._compressedCapsCache;
    }

    _buildWebglDrawBuffers() {
        // Enum table from the WEBGL_draw_buffers extension spec.
        // The numeric values match the GLES 3.0 core enums
        // (`GL_COLOR_ATTACHMENT0_WEBGL == GL_COLOR_ATTACHMENT0`),
        // so we can forward the untransformed buffer list straight
        // to `op_draw_buffers`.
        const ctx = this;
        const obj = {
            COLOR_ATTACHMENT0_WEBGL: 0x8CE0,
            COLOR_ATTACHMENT1_WEBGL: 0x8CE1,
            COLOR_ATTACHMENT2_WEBGL: 0x8CE2,
            COLOR_ATTACHMENT3_WEBGL: 0x8CE3,
            COLOR_ATTACHMENT4_WEBGL: 0x8CE4,
            COLOR_ATTACHMENT5_WEBGL: 0x8CE5,
            COLOR_ATTACHMENT6_WEBGL: 0x8CE6,
            COLOR_ATTACHMENT7_WEBGL: 0x8CE7,
            COLOR_ATTACHMENT8_WEBGL: 0x8CE8,
            COLOR_ATTACHMENT9_WEBGL: 0x8CE9,
            COLOR_ATTACHMENT10_WEBGL: 0x8CEA,
            COLOR_ATTACHMENT11_WEBGL: 0x8CEB,
            COLOR_ATTACHMENT12_WEBGL: 0x8CEC,
            COLOR_ATTACHMENT13_WEBGL: 0x8CED,
            COLOR_ATTACHMENT14_WEBGL: 0x8CEE,
            COLOR_ATTACHMENT15_WEBGL: 0x8CEF,
            DRAW_BUFFER0_WEBGL: 0x8825,
            DRAW_BUFFER1_WEBGL: 0x8826,
            DRAW_BUFFER2_WEBGL: 0x8827,
            DRAW_BUFFER3_WEBGL: 0x8828,
            DRAW_BUFFER4_WEBGL: 0x8829,
            DRAW_BUFFER5_WEBGL: 0x882A,
            DRAW_BUFFER6_WEBGL: 0x882B,
            DRAW_BUFFER7_WEBGL: 0x882C,
            DRAW_BUFFER8_WEBGL: 0x882D,
            DRAW_BUFFER9_WEBGL: 0x882E,
            DRAW_BUFFER10_WEBGL: 0x882F,
            DRAW_BUFFER11_WEBGL: 0x8830,
            DRAW_BUFFER12_WEBGL: 0x8831,
            DRAW_BUFFER13_WEBGL: 0x8832,
            DRAW_BUFFER14_WEBGL: 0x8833,
            DRAW_BUFFER15_WEBGL: 0x8834,
            MAX_COLOR_ATTACHMENTS_WEBGL: 0x8CDF,
            MAX_DRAW_BUFFERS_WEBGL: 0x8824,
            drawBuffersWEBGL(buffers) {
                const buf = new Uint32Array(buffers);
                _rawDrawBuffers(ctx._canvasId, buf);
            },
        };
        return obj;
    }

    _buildCompressedEtc() {
        // ETC2/EAC format enum block.  No methods - data upload
        // goes through `compressedTexImage2D` like every other
        // compressed extension.  Values mirror the GLES 3.0 core
        // internal-format constants so our existing
        // `op_compressed_tex_image_2d` accepts them unchanged.
        return {
            COMPRESSED_R11_EAC: 0x9270,
            COMPRESSED_SIGNED_R11_EAC: 0x9271,
            COMPRESSED_RG11_EAC: 0x9272,
            COMPRESSED_SIGNED_RG11_EAC: 0x9273,
            COMPRESSED_RGB8_ETC2: 0x9274,
            COMPRESSED_SRGB8_ETC2: 0x9275,
            COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2: 0x9276,
            COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2: 0x9277,
            COMPRESSED_RGBA8_ETC2_EAC: 0x9278,
            COMPRESSED_SRGB8_ALPHA8_ETC2_EAC: 0x9279,
        };
    }

    _buildCompressedAstc() {
        // ASTC LDR format block.  Included enums are the subset the
        // compressed upload path accepts (see
        // `graphics/compressed_upload.rs::CompressedFormat`).  Games
        // that query the full ASTC enum table get the 4x4 / 6x6 /
        // 8x8 blocks we actually decode.
        return {
            COMPRESSED_RGBA_ASTC_4x4_KHR: 0x93B0,
            COMPRESSED_RGBA_ASTC_5x4_KHR: 0x93B1,
            COMPRESSED_RGBA_ASTC_5x5_KHR: 0x93B2,
            COMPRESSED_RGBA_ASTC_6x5_KHR: 0x93B3,
            COMPRESSED_RGBA_ASTC_6x6_KHR: 0x93B4,
            COMPRESSED_RGBA_ASTC_8x5_KHR: 0x93B5,
            COMPRESSED_RGBA_ASTC_8x6_KHR: 0x93B6,
            COMPRESSED_RGBA_ASTC_8x8_KHR: 0x93B7,
            COMPRESSED_RGBA_ASTC_10x5_KHR: 0x93B8,
            COMPRESSED_RGBA_ASTC_10x6_KHR: 0x93B9,
            COMPRESSED_RGBA_ASTC_10x8_KHR: 0x93BA,
            COMPRESSED_RGBA_ASTC_10x10_KHR: 0x93BB,
            COMPRESSED_RGBA_ASTC_12x10_KHR: 0x93BC,
            COMPRESSED_RGBA_ASTC_12x12_KHR: 0x93BD,
            COMPRESSED_SRGB8_ALPHA8_ASTC_4x4_KHR: 0x93D0,
            COMPRESSED_SRGB8_ALPHA8_ASTC_6x6_KHR: 0x93D4,
            COMPRESSED_SRGB8_ALPHA8_ASTC_8x8_KHR: 0x93D7,
        };
    }

    _buildAngleInstancedArrays() {
        const ctx = this;
        return {
            // Published enum from the ANGLE_instanced_arrays spec.
            VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE: 0x88FE,
            drawArraysInstancedANGLE(mode, first, count, primcount) {
                // Encode if all params are numbers; otherwise flush+raw.
                if (typeof mode === "number" && typeof first === "number" &&
                    typeof count === "number" && typeof primcount === "number") {
                    encodeDrawArraysInstanced(
                        ctx._canvasId, mode >>> 0, first | 0, count | 0, primcount | 0,
                    );
                } else {
                    flushGlCommandStream();
                    op_draw_arrays_instanced(
                        ctx._canvasId, mode, first, count, primcount,
                    );
                }
            },
            drawElementsInstancedANGLE(mode, count, type, offset, primcount) {
                if (typeof mode === "number" && typeof count === "number" &&
                    typeof type === "number" && typeof offset === "number" &&
                    typeof primcount === "number") {
                    encodeDrawElementsInstanced(
                        ctx._canvasId, mode >>> 0, count | 0, type >>> 0, offset | 0, primcount | 0,
                    );
                } else {
                    flushGlCommandStream();
                    op_draw_elements_instanced(
                        ctx._canvasId, mode, count, type, offset, primcount,
                    );
                }
            },
            vertexAttribDivisorANGLE(index, divisor) {
                if (typeof index === "number" && typeof divisor === "number") {
                    encodeVertexAttribDivisor(ctx._canvasId, index >>> 0, divisor >>> 0);
                } else {
                    flushGlCommandStream();
                    op_vertex_attrib_divisor(ctx._canvasId, index, divisor);
                }
            },
        };
    }

    _buildOesVertexArrayObject() {
        const ctx = this;
        return {
            VERTEX_ARRAY_BINDING_OES: 0x85B5,
            createVertexArrayOES() {
                const id = nextResourceId();
                _rawCreateVertexArray(ctx._canvasId, id);
                return { _id: id, _kind: 'vao' };
            },
            deleteVertexArrayOES(vao) {
                if (vao && vao._id) _rawDeleteVertexArray(vao._id);
            },
            isVertexArrayOES(vao) {
                return !!(vao && typeof vao._id === 'number' && vao._kind === 'vao');
            },
            bindVertexArrayOES(vao) {
                // opcode 14: H C U. vaoId is u32 (0 = unbind).
                const vaoId = vao ? vao._id : 0;
                if (typeof vaoId === "number") {
                    encodeBindVertexArray(ctx._canvasId, vaoId >>> 0);
                } else {
                    flushGlCommandStream();
                    op_bind_vertex_array(ctx._canvasId, vaoId);
                }
            },
        };
    }

    // -- Phase 1B: Textures --

    createTexture() {
        const id = nextResourceId();
        _rawCreateTexture(this._canvasId, id);
        return new WebglObject(id);
    }

    deleteTexture(texture) {
        if (texture && texture.id !== undefined) _rawDeleteTexture(texture.id);
    }

    bindTexture(target, texture) {
        const tex = texture || null;
        if (target === 0x0de1) this._textureBindings2D.set(this._activeTextureUnit, tex); // TEXTURE_2D
        else if (target === 0x8513) this._textureBindingsCube.set(this._activeTextureUnit, tex); // TEXTURE_CUBE_MAP
        const texId = tex ? tex.id : -1;
        // opcode 10: H C U I. target is u32, texId is i32 (negative = unbind).
        if (typeof target === "number" && typeof texId === "number") {
            encodeBindTexture(this._canvasId, target >>> 0, texId | 0);
            return;
        }
        flushGlCommandStream();
        op_bind_texture(this._canvasId, target, texId);
    }

    activeTexture(unit) {
        this._activeTextureUnit = unit;
        // opcode 11: H C U.
        if (typeof unit === "number") {
            encodeActiveTexture(this._canvasId, unit >>> 0);
            return;
        }
        flushGlCommandStream();
        op_active_texture(this._canvasId, unit);
    }

    texImage2D(target, level, internalformat, a4, a5, a6, a7, a8, a9) {
        // 9-arg: (target, level, internalformat, width, height, border, format, type, pixels)
        // 6-arg: (target, level, internalformat, format, type, source)
        if (a7 !== undefined) {
            // Text texture cache hit takes precedence.
            if (_migoTexImageFromTextCache(this._canvasId, target, level, internalformat, a9)) {
                return;
            }
            const snapshotId =
                a9 && typeof a9 === "object" && (a9.__migo_snapshot_id__ | 0);
            if (
                snapshotId &&
                snapshotId !== 0 &&
                a9.width === a4 &&
                a9.height === a5
            ) {
                _rawTexImage2DFromSnapshot(
                    this._canvasId, target, level, internalformat, a7, a8, snapshotId,
                );
                return;
            }
            if (_migoIsHTMLCanvas(a9) && a9.width === a4 && a9.height === a5) {
                const ctx9 = a9._context;
                if (ctx9 && typeof ctx9._consumeTextCacheForTexImage === "function"
                        && ctx9._consumeTextCacheForTexImage(
                            this._canvasId, target, level, internalformat)) {
                    return;
                }
                _rawTexImage2DFromCanvas2d(
                    this._canvasId, target, level, internalformat, a9._rid, 0, 0, a4 | 0, a5 | 0,
                );
                return;
            }
            const data = a9 != null ? toUnit8Array(a9) : null;
            _rawTexImage2D(this._canvasId, target, level, internalformat, a4, a5, a6, a7, a8, data);
        } else {
            const source = a6;
            if (_migoTexImageFromTextCache(this._canvasId, target, level, internalformat, source)) {
                return;
            }
            const snapshotId =
                source && typeof source === "object"
                    ? (source.__migo_snapshot_id__ | 0)
                    : 0;
            if (snapshotId !== 0) {
                _rawTexImage2DFromSnapshot(
                    this._canvasId, target, level, internalformat, a4, a5, snapshotId,
                );
                return;
            }
            if (_migoIsHTMLCanvas(source)) {
                const cw = source.width | 0;
                const ch = source.height | 0;
                if (cw > 0 && ch > 0) {
                    const ctx6 = source._context;
                    if (ctx6 && typeof ctx6._consumeTextCacheForTexImage === "function"
                            && ctx6._consumeTextCacheForTexImage(
                                this._canvasId, target, level, internalformat)) {
                        return;
                    }
                    _rawTexImage2DFromCanvas2d(
                        this._canvasId, target, level, internalformat,
                        source._rid, 0, 0, cw, ch,
                    );
                    return;
                }
            }
            const imageId = source && typeof source.rid === "number" ? source.rid : null;
            if (imageId != null) {
                _rawTexImage2DFromImage(this._canvasId, target, level, internalformat, a4, a5, imageId);
            } else {
                const raw = sourceToRawRgba(source);
                if (raw) {
                    _rawTexImage2D(
                        this._canvasId, target, level, internalformat,
                        raw.width, raw.height, 0, a4, a5, raw.data,
                    );
                    return;
                }
                const kind = source && source.constructor ? source.constructor.name : typeof source;
                console.warn(`texImage2D 6-argument form unsupported source: ${kind}`);
            }
        }
    }

    texSubImage2D(target, level, xoffset, yoffset, width, height, format, type, pixels) {
        // 9-arg: (..., width, height, format, type, pixels)
        if (pixels !== undefined) {
            if (pixels == null) return;
            const snapshotId =
                pixels && typeof pixels === "object"
                    ? (pixels.__migo_snapshot_id__ | 0)
                    : 0;
            if (snapshotId !== 0 && pixels.width === width && pixels.height === height) {
                _rawTexSubImage2DFromSnapshot(
                    this._canvasId, target, level, xoffset, yoffset, format, type, snapshotId,
                );
                return;
            }
            if (_migoIsHTMLCanvas(pixels) && pixels.width === width && pixels.height === height) {
                _rawTexSubImage2DFromCanvas2d(
                    this._canvasId, target, level, xoffset, yoffset,
                    pixels._rid, 0, 0, width | 0, height | 0,
                );
                return;
            }
            const data = toUnit8Array(pixels);
            _rawTexSubImage2D(this._canvasId, target, level, xoffset, yoffset, width, height, format, type, data);
            return;
        }

        // 7-arg: (..., format, type, source)
        const source = format;
        const sourceFormat = width;
        const sourceType = height;
        const subSnapshotId =
            source && typeof source === "object"
                ? (source.__migo_snapshot_id__ | 0)
                : 0;
        if (subSnapshotId !== 0) {
            _rawTexSubImage2DFromSnapshot(
                this._canvasId, target, level, xoffset, yoffset, sourceFormat, sourceType, subSnapshotId,
            );
            return;
        }
        if (_migoIsHTMLCanvas(source)) {
            const cw = source.width | 0;
            const ch = source.height | 0;
            if (cw > 0 && ch > 0) {
                _rawTexSubImage2DFromCanvas2d(
                    this._canvasId, target, level, xoffset, yoffset,
                    source._rid, 0, 0, cw, ch,
                );
                return;
            }
        }
        const imageId = source && typeof source.rid === "number" ? source.rid : null;
        if (imageId != null) {
            _rawTexSubImage2DFromImage(
                this._canvasId, target, level, xoffset, yoffset, sourceFormat, sourceType, imageId,
            );
        } else {
            const raw = sourceToRawRgba(source);
            if (raw) {
                _rawTexSubImage2D(
                    this._canvasId, target, level, xoffset, yoffset,
                    raw.width, raw.height, sourceFormat, sourceType, raw.data,
                );
                return;
            }
            const kind = source && source.constructor ? source.constructor.name : typeof source;
            console.warn(`texSubImage2D 7-argument form unsupported source: ${kind}`);
        }
    }

    texParameteri(target, pname, param) {
        // opcode 40: H C U U I. target/pname are u32, param is i32.
        if (typeof target === "number" && typeof pname === "number" &&
            typeof param === "number") {
            encodeTexParameteri(this._canvasId, target >>> 0, pname >>> 0, param | 0);
            return;
        }
        flushGlCommandStream();
        _rawTexParameteri(this._canvasId, target, pname, param);
    }

    texParameterf(target, pname, param) {
        // opcode 41: H C U U F. target/pname are u32, param is f32.
        if (typeof target === "number" && typeof pname === "number" &&
            typeof param === "number") {
            encodeTexParameterf(this._canvasId, target >>> 0, pname >>> 0, param);
            return;
        }
        _rawTexParameterf(this._canvasId, target, pname, param);
    }

    generateMipmap(target) {
        // opcode 42: H C U.
        if (typeof target === "number") {
            encodeGenerateMipmap(this._canvasId, target >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawGenerateMipmap(this._canvasId, target);
    }

    pixelStorei(pname, param) {
        // UNPACK_FLIP_Y_WEBGL / UNPACK_PREMULTIPLY_ALPHA_WEBGL / UNPACK_COLORSPACE_CONVERSION_WEBGL
        // are JS-only state; colorspace is a no-op.
        if (pname === 0x9240) { this._unpackFlipY = !!param; return; }
        if (pname === 0x9241) { this._unpackPremultiplyAlpha = !!param; return; }
        if (pname === 0x9243) { return; }
        let value;
        if (param === true) value = 1;
        else if (param === false) value = 0;
        else value = Number(param) | 0;
        // opcode 43: H C U I. pname is u32, value is i32.
        if (typeof pname === "number") {
            encodePixelStorei(this._canvasId, pname >>> 0, value);
            return;
        }
        flushGlCommandStream();
        _rawPixelStorei(this._canvasId, pname, value);
    }

    compressedTexImage2D(target, level, internalformat, width, height, border, data) {
        const u8 = toUnit8Array(data);
        _rawCompressedTexImage2D(this._canvasId, target, level, internalformat, width, height, border, u8);
    }

    compressedTexSubImage2D(target, level, xoffset, yoffset, width, height, format, data) {
        const u8 = toUnit8Array(data);
        _rawCompressedTexSubImage2D(this._canvasId, target, level, xoffset, yoffset, width, height, format, u8);
    }

    // -- Phase 1C: Buffer & Vertex Extensions --

    bufferSubData(target, offset, data) {
        const u8 = toUnit8Array(data);
        _rawBufferSubData(this._canvasId, target, offset, u8);
    }

    disableVertexAttribArray(index) {
        // opcode 17: H C U.
        if (typeof index === "number") {
            encodeDisableVertexAttribArray(this._canvasId, index >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawDisableVertexAttribArray(this._canvasId, index);
    }

    clearDepth(depth) {
        // opcode 4: H C F.
        if (typeof depth === "number") {
            encodeClearDepth(this._canvasId, depth);
            return;
        }
        _rawClearDepth(this._canvasId, depth);
    }

    clearStencil(s) {
        // opcode 5: H C I.
        if (typeof s === "number") {
            encodeClearStencil(this._canvasId, s | 0);
            return;
        }
        flushGlCommandStream();
        _rawClearStencil(this._canvasId, s);
    }

    // -- Phase 2A: Blend/Depth/Stencil/Cull State --

    blendFunc(sfactor, dfactor) {
        if (typeof sfactor === "number" && typeof dfactor === "number") {
            encodeBlendFunc(this._canvasId, sfactor >>> 0, dfactor >>> 0);
        } else {
            flushGlCommandStream();
            _rawBlendFunc(this._canvasId, sfactor, dfactor);
        }
    }
    blendFuncSeparate(srcRGB, dstRGB, srcAlpha, dstAlpha) {
        if (typeof srcRGB === "number" && typeof dstRGB === "number" &&
            typeof srcAlpha === "number" && typeof dstAlpha === "number") {
            encodeBlendFuncSeparate(this._canvasId, srcRGB >>> 0, dstRGB >>> 0, srcAlpha >>> 0, dstAlpha >>> 0);
        } else {
            flushGlCommandStream();
            _rawBlendFuncSeparate(this._canvasId, srcRGB, dstRGB, srcAlpha, dstAlpha);
        }
    }
    blendEquation(mode) {
        if (typeof mode === "number") {
            encodeBlendEquation(this._canvasId, mode >>> 0);
        } else {
            flushGlCommandStream();
            _rawBlendEquation(this._canvasId, mode);
        }
    }
    blendEquationSeparate(modeRGB, modeAlpha) {
        if (typeof modeRGB === "number" && typeof modeAlpha === "number") {
            encodeBlendEquationSeparate(this._canvasId, modeRGB >>> 0, modeAlpha >>> 0);
        } else {
            flushGlCommandStream();
            _rawBlendEquationSeparate(this._canvasId, modeRGB, modeAlpha);
        }
    }
    blendColor(r, g, b, a) {
        if (typeof r === "number" && typeof g === "number" &&
            typeof b === "number" && typeof a === "number") {
            encodeBlendColor(this._canvasId, r, g, b, a);
        } else {
            _rawBlendColor(this._canvasId, r, g, b, a);
        }
    }
    depthFunc(func) {
        if (typeof func === "number") {
            encodeDepthFunc(this._canvasId, func >>> 0);
        } else {
            flushGlCommandStream();
            _rawDepthFunc(this._canvasId, func);
        }
    }
    depthMask(flag) {
        if (typeof flag === "boolean") {
            encodeDepthMask(this._canvasId, flag);
        } else {
            flushGlCommandStream();
            op_depth_mask(this._canvasId, flag);
        }
    }
    depthRange(near, far) {
        if (typeof near === "number" && typeof far === "number") {
            encodeDepthRange(this._canvasId, near, far);
        } else {
            _rawDepthRange(this._canvasId, near, far);
        }
    }
    stencilFunc(func, ref_, mask) {
        if (typeof func === "number" && typeof ref_ === "number" && typeof mask === "number") {
            encodeStencilFunc(this._canvasId, func >>> 0, ref_ | 0, mask >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilFunc(this._canvasId, func, ref_, mask);
        }
    }
    stencilFuncSeparate(face, func, ref_, mask) {
        if (typeof face === "number" && typeof func === "number" &&
            typeof ref_ === "number" && typeof mask === "number") {
            encodeStencilFuncSeparate(this._canvasId, face >>> 0, func >>> 0, ref_ | 0, mask >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilFuncSeparate(this._canvasId, face, func, ref_, mask);
        }
    }
    stencilOp(fail, zfail, zpass) {
        if (typeof fail === "number" && typeof zfail === "number" && typeof zpass === "number") {
            encodeStencilOp(this._canvasId, fail >>> 0, zfail >>> 0, zpass >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilOp(this._canvasId, fail, zfail, zpass);
        }
    }
    stencilOpSeparate(face, fail, zfail, zpass) {
        if (typeof face === "number" && typeof fail === "number" &&
            typeof zfail === "number" && typeof zpass === "number") {
            encodeStencilOpSeparate(this._canvasId, face >>> 0, fail >>> 0, zfail >>> 0, zpass >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilOpSeparate(this._canvasId, face, fail, zfail, zpass);
        }
    }
    stencilMask(mask) {
        if (typeof mask === "number") {
            encodeStencilMask(this._canvasId, mask >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilMask(this._canvasId, mask);
        }
    }
    stencilMaskSeparate(face, mask) {
        if (typeof face === "number" && typeof mask === "number") {
            encodeStencilMaskSeparate(this._canvasId, face >>> 0, mask >>> 0);
        } else {
            flushGlCommandStream();
            _rawStencilMaskSeparate(this._canvasId, face, mask);
        }
    }
    cullFace(mode) {
        if (typeof mode === "number") {
            encodeCullFace(this._canvasId, mode >>> 0);
        } else {
            flushGlCommandStream();
            _rawCullFace(this._canvasId, mode);
        }
    }
    frontFace(mode) {
        if (typeof mode === "number") {
            encodeFrontFace(this._canvasId, mode >>> 0);
        } else {
            flushGlCommandStream();
            _rawFrontFace(this._canvasId, mode);
        }
    }
    colorMask(r, g, b, a) {
        if (typeof r === "boolean" && typeof g === "boolean" &&
            typeof b === "boolean" && typeof a === "boolean") {
            encodeColorMask(this._canvasId, r, g, b, a);
        } else {
            flushGlCommandStream();
            op_color_mask(this._canvasId, r, g, b, a);
        }
    }
    scissor(x, y, width, height) {
        // opcode 37: H C I I I I.
        if (typeof x === "number" && typeof y === "number" &&
            typeof width === "number" && typeof height === "number") {
            encodeScissor(this._canvasId, x | 0, y | 0, width | 0, height | 0);
        } else {
            flushGlCommandStream();
            _rawScissor(this._canvasId, x, y, width, height);
        }
    }
    lineWidth(width) {
        if (typeof width === "number") {
            encodeLineWidth(this._canvasId, width);
        } else {
            _rawLineWidth(this._canvasId, width);
        }
    }
    polygonOffset(factor, units) {
        if (typeof factor === "number" && typeof units === "number") {
            encodePolygonOffset(this._canvasId, factor, units);
        } else {
            _rawPolygonOffset(this._canvasId, factor, units);
        }
    }

    // -- Phase 2B: Uniform Variants --

    uniform1i(location, x) {
        // WebGL coerces the value (WebIDL GLint) -- engines pass booleans for
        // `uniform bool` samplers/flags (e.g. Phaser: `uniform1i(loc, true)`).
        // `| 0` applies ToInt32 (true -> 1, false -> 0), matching the browser.
        // opcode 54: H C I I. location is i32, x is i32.
        const loc = _loc(location);
        const xi = x | 0;
        encodeUniform1i(this._canvasId, loc, xi);
    }
    uniform1f(location, x) {
        // opcode 55: H C I F. Always encodable (f32 accepts any number).
        encodeUniform1f(this._canvasId, _loc(location), +x);
    }
    uniform2f(location, x, y) {
        // opcode 56: H C I F F.
        encodeUniform2f(this._canvasId, _loc(location), +x, +y);
    }
    uniform4f(location, x, y, z, w) {
        // opcode 58: H C I F F F F.
        encodeUniform4f(this._canvasId, _loc(location), +x, +y, +z, +w);
    }
    uniform1iv(location, value) {
        const loc = _loc(location);
        const payload = toInt32AsUint32(value);
        if (!encodeUniform1iv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform1iv(this._canvasId, loc, payload);
        }
    }
    uniform1fv(location, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (!encodeUniform1fv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform1fv(this._canvasId, loc, payload);
        }
    }
    uniform2iv(location, value) {
        const loc = _loc(location);
        const payload = toInt32AsUint32(value);
        if (!encodeUniform2iv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform2iv(this._canvasId, loc, payload);
        }
    }
    uniform2fv(location, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (!encodeUniform2fv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform2fv(this._canvasId, loc, payload);
        }
    }
    uniform3iv(location, value) {
        const loc = _loc(location);
        const payload = toInt32AsUint32(value);
        if (!encodeUniform3iv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform3iv(this._canvasId, loc, payload);
        }
    }
    uniform3fv(location, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (!encodeUniform3fv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform3fv(this._canvasId, loc, payload);
        }
    }
    uniform4iv(location, value) {
        const loc = _loc(location);
        const payload = toInt32AsUint32(value);
        if (!encodeUniform4iv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform4iv(this._canvasId, loc, payload);
        }
    }
    uniform4fv(location, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (!encodeUniform4fv(this._canvasId, loc, payload)) {
            flushGlCommandStream();
            _rawUniform4fv(this._canvasId, loc, payload);
        }
    }
    uniformMatrix2fv(location, transpose, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (typeof transpose !== "boolean" ||
            !encodeUniformMatrix2fv(this._canvasId, loc, transpose, payload)) {
            flushGlCommandStream();
            op_uniform_matrix_2fv(this._canvasId, loc, transpose, payload);
        }
    }
    uniformMatrix4fv(location, transpose, value) {
        const loc = _loc(location);
        const payload = toFloat32AsUint32(value);
        if (typeof transpose !== "boolean" ||
            !encodeUniformMatrix4fv(this._canvasId, loc, transpose, payload)) {
            flushGlCommandStream();
            op_uniform_matrix_4fv(this._canvasId, loc, transpose, payload);
        }
    }

    // -- Phase 3A: Framebuffer/Renderbuffer --

    createFramebuffer() {
        const id = nextResourceId();
        _rawCreateFramebuffer(this._canvasId, id);
        return new WebglObject(id);
    }
    deleteFramebuffer(fb) {
        if (fb && fb.id !== undefined) _rawDeleteFramebuffer(fb.id);
    }
    bindFramebuffer(target, fb) {
        this._framebufferBinding = fb || null;
        const fbId = fb ? fb.id : -1;
        // opcode 12: H C U I.
        if (typeof target === "number" && typeof fbId === "number") {
            encodeBindFramebuffer(this._canvasId, target >>> 0, fbId | 0);
            return;
        }
        flushGlCommandStream();
        _rawBindFramebuffer(this._canvasId, target, fbId);
    }
    framebufferTexture2D(target, attachment, textarget, texture, level) {
        _rawFramebufferTexture2D(this._canvasId, target, attachment, textarget, texture ? texture.id : -1, level);
    }
    framebufferRenderbuffer(target, attachment, renderbuffertarget, renderbuffer) {
        _rawFramebufferRenderbuffer(this._canvasId, target, attachment, renderbuffertarget, renderbuffer ? renderbuffer.id : -1);
    }
    checkFramebufferStatus(target) {
        return _rawCheckFramebufferStatus(this._canvasId, target);
    }
    createRenderbuffer() {
        const id = nextResourceId();
        _rawCreateRenderbuffer(this._canvasId, id);
        return new WebglObject(id);
    }
    deleteRenderbuffer(rb) {
        if (rb && rb.id !== undefined) _rawDeleteRenderbuffer(rb.id);
    }
    bindRenderbuffer(target, rb) {
        this._renderbufferBinding = rb || null;
        const rbId = rb ? rb.id : -1;
        // opcode 13: H C U I.
        if (typeof target === "number" && typeof rbId === "number") {
            encodeBindRenderbuffer(this._canvasId, target >>> 0, rbId | 0);
            return;
        }
        flushGlCommandStream();
        _rawBindRenderbuffer(this._canvasId, target, rbId);
    }
    renderbufferStorage(target, internalformat, width, height) {
        _rawRenderbufferStorage(this._canvasId, target, internalformat, width, height);
    }

    // -- Phase 3B: Misc --

    readPixels(x, y, width, height, format, type, pixels) {
        const data = _rawReadPixels(this._canvasId, x, y, width, height, format, type);
        if (data && pixels) {
            const u8 = new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength);
            u8.set(data.subarray(0, u8.length));
        }
    }
    hint(target, mode) {
        // opcode 44: H C U U.
        if (typeof target === "number" && typeof mode === "number") {
            encodeHint(this._canvasId, target >>> 0, mode >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawHint(this._canvasId, target, mode);
    }
}

Object.assign(WebGLRenderingContext.prototype, WebglConstants);

/**
 * WebGL 2.0 facade.  Extends `WebGLRenderingContext` with the ES 3.0
 * additions backed by the handler in
 * engine/crates/graphics/renderergl/handler.rs and the op wrappers in
 * engine/crates/runtime-v8/rendering/webgl/webgl.rs.
 *
 * Minimum footprint: VAO, instancing, UBO, sampler objects, sync
 * objects, immutable texture storage, BlitFramebuffer,
 * InvalidateFramebuffer, MSAA renderbuffers, and multiple draw/read
 * buffers.  More advanced features (Transform Feedback, Query Objects,
 * texImage3D, compressedTexSubImage3D, ...) will be added on demand as
 * real-world games request them -- they all share the same GLCmd + op
 * + handler layer as the items above.
 */
class WebGL2RenderingContext extends WebGLRenderingContext {
    constructor(canvas) {
        super(canvas);
        this._queryRegistry = new Map();
        this._currentQueryByTarget = new Map();
        this._tfRegistry = new Map();
        this._currentTransformFeedback = null;
    }

    // ---- Vertex Array Objects ----------------------------------
    createVertexArray() {
        // op_alloc_gl_resource_id: direct, no-submit.
        const id = op_alloc_gl_resource_id_webgl2();
        _rawCreateVertexArray(this._canvasId, id);
        return { _id: id, _kind: 'vao' };
    }
    deleteVertexArray(vao) {
        if (vao && vao._id) _rawDeleteVertexArray(vao._id);
    }
    bindVertexArray(vao) {
        // opcode 14: H C U. vaoId is u32 (0 = unbind).
        const vaoId = vao ? vao._id : 0;
        if (typeof vaoId === "number") {
            encodeBindVertexArray(this._canvasId, vaoId >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawBindVertexArray(this._canvasId, vaoId);
    }

    // ---- Instanced drawing -------------------------------------
    vertexAttribDivisor(index, divisor) {
        // opcode 19: H C U U.
        if (typeof index === "number" && typeof divisor === "number") {
            encodeVertexAttribDivisor(this._canvasId, index >>> 0, divisor >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawVertexAttribDivisor(this._canvasId, index, divisor);
    }
    drawArraysInstanced(mode, first, count, instanceCount) {
        // opcode 49: H C U I I I.
        if (typeof mode === "number" && typeof first === "number" &&
            typeof count === "number" && typeof instanceCount === "number") {
            encodeDrawArraysInstanced(this._canvasId, mode >>> 0, first | 0, count | 0, instanceCount | 0);
            return;
        }
        flushGlCommandStream();
        _rawDrawArraysInstanced(this._canvasId, mode, first, count, instanceCount);
    }
    drawElementsInstanced(mode, count, type, offset, instanceCount) {
        // opcode 50: H C U I U I I.
        if (typeof mode === "number" && typeof count === "number" &&
            typeof type === "number" && typeof offset === "number" &&
            typeof instanceCount === "number") {
            encodeDrawElementsInstanced(this._canvasId, mode >>> 0, count | 0, type >>> 0, offset | 0, instanceCount | 0);
            return;
        }
        flushGlCommandStream();
        _rawDrawElementsInstanced(this._canvasId, mode, count, type, offset, instanceCount);
    }

    // ---- Uniform Buffer Objects --------------------------------
    getUniformBlockIndex(program, name) {
        return _rawGetUniformBlockIndex(program._id, name);
    }
    uniformBlockBinding(program, uniformBlockIndex, uniformBlockBinding) {
        _rawUniformBlockBinding(program._id, uniformBlockIndex, uniformBlockBinding);
    }
    bindBufferBase(target, index, buffer) {
        // opcode 51: H C U U U. target/index are u32, bufferId is u32 (0 = unbind).
        const bufferId = buffer ? buffer.id ?? buffer._id : 0;
        if (typeof target === "number" && typeof index === "number" && typeof bufferId === "number") {
            encodeBindBufferBase(this._canvasId, target >>> 0, index >>> 0, bufferId >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawBindBufferBase(this._canvasId, target, index, bufferId);
    }
    bindBufferRange(target, index, buffer, offset, size) {
        // opcode 52: H C U U U I I.
        const bufferId = buffer ? buffer.id ?? buffer._id : 0;
        if (typeof target === "number" && typeof index === "number" &&
            typeof bufferId === "number" && typeof offset === "number" && typeof size === "number") {
            encodeBindBufferRange(this._canvasId, target >>> 0, index >>> 0, bufferId >>> 0, offset | 0, size | 0);
            return;
        }
        flushGlCommandStream();
        _rawBindBufferRange(this._canvasId, target, index, bufferId, offset, size);
    }

    // ---- Immutable texture storage ------------------------------
    texStorage2D(target, levels, internalformat, width, height) {
        _rawTexStorage2D(this._canvasId, target, levels, internalformat, width, height);
    }

    // ---- Framebuffer ops ---------------------------------------
    blitFramebuffer(srcX0, srcY0, srcX1, srcY1, dstX0, dstY0, dstX1, dstY1, mask, filter) {
        _rawBlitFramebuffer(this._canvasId, srcX0, srcY0, srcX1, srcY1,
                             dstX0, dstY0, dstX1, dstY1, mask, filter);
    }
    invalidateFramebuffer(target, attachments) {
        // WebGL spec accepts a sequence<GLenum>; normalise to Uint32Array
        // for the op boundary.
        const buf = attachments instanceof Uint32Array
            ? attachments
            : new Uint32Array(attachments || []);
        _rawInvalidateFramebuffer(this._canvasId, target, buf);
    }
    renderbufferStorageMultisample(target, samples, internalformat, width, height) {
        _rawRenderbufferStorageMultisample(this._canvasId, target, samples,
                                            internalformat, width, height);
    }

    // ---- Sampler objects ---------------------------------------
    createSampler() {
        // op_alloc_gl_resource_id: direct, no-submit.
        const id = op_alloc_gl_resource_id_webgl2();
        _rawCreateSampler(this._canvasId, id);
        return { _id: id, _kind: 'sampler' };
    }
    deleteSampler(sampler) {
        if (sampler && sampler._id) _rawDeleteSampler(sampler._id);
    }
    bindSampler(unit, sampler) {
        // opcode 15: H C U U. unit is u32, samplerId is u32 (0 = unbind).
        const samplerId = sampler ? sampler._id : 0;
        if (typeof unit === "number" && typeof samplerId === "number") {
            encodeBindSampler(this._canvasId, unit >>> 0, samplerId >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawBindSampler(this._canvasId, unit, samplerId);
    }
    samplerParameteri(sampler, pname, param) {
        // opcode 45: H U U I. samplerId is u32, pname is u32, param is i32.
        // No canvas field: sampler is a global resource identified by its id.
        if (!sampler || !sampler._id) return;
        if (typeof pname === "number" && typeof param === "number") {
            encodeSamplerParameteri(sampler._id >>> 0, pname >>> 0, param | 0);
            return;
        }
        flushGlCommandStream();
        _rawSamplerParameteri(sampler._id, pname, param);
    }
    samplerParameterf(sampler, pname, param) {
        // opcode 46: H U U F. samplerId is u32, pname is u32, param is f32.
        // No canvas field: sampler is a global resource identified by its id.
        if (!sampler || !sampler._id) return;
        if (typeof pname === "number" && typeof param === "number") {
            encodeSamplerParameterf(sampler._id >>> 0, pname >>> 0, param);
            return;
        }
        _rawSamplerParameterf(sampler._id, pname, param);
    }

    // ---- Fence syncs -------------------------------------------
    fenceSync(condition, flags) {
        // op_alloc_gl_resource_id: direct, no-submit.
        const id = op_alloc_gl_resource_id_webgl2();
        _rawFenceSync(this._canvasId, id, condition, flags);
        return { _id: id, _kind: 'sync' };
    }
    deleteSync(sync) {
        if (sync && sync._id) _rawDeleteSync(sync._id);
    }
    /**
     * clientWaitSync(sync, flags, timeout) -- WebGL spec says timeout is
     * an Int64.  JS numbers max out at 2^53, which is larger than GLES'
     * effective timeout range so a `number` is safe here.  Pass `0` to
     * poll without blocking.
     */
    clientWaitSync(sync, flags, timeout) {
        if (!sync || !sync._id) return 0x911D; // WAIT_FAILED
        return _rawClientWaitSync(sync._id, flags, Number(timeout) || 0);
    }

    // ---- Draw / read buffer selection --------------------------
    drawBuffers(buffers) {
        const buf = buffers instanceof Uint32Array
            ? buffers
            : new Uint32Array(buffers || []);
        _rawDrawBuffers(this._canvasId, buf);
    }
    readBuffer(src) {
        // opcode 53: H C U.
        if (typeof src === "number") {
            encodeReadBuffer(this._canvasId, src >>> 0);
            return;
        }
        flushGlCommandStream();
        _rawReadBuffer(this._canvasId, src);
    }

    // ---- Query objects -----------------------------------------
    createQuery() {
        // op_alloc_gl_resource_id: direct, no-submit.
        const id = op_alloc_gl_resource_id_webgl2();
        _rawCreateQuery(this._canvasId, id);
        const query = { _id: id, _kind: 'query' };
        this._queryRegistry.set(id, {
            active: false,
            boundOnce: false,
            deleted: false,
            target: 0,
        });
        return query;
    }
    deleteQuery(query) {
        if (!query || !query._id) return;
        const state = this._queryRegistry.get(query._id);
        if (!state || state.deleted) return;
        state.deleted = true;
        state.active = false;
        if (this._currentQueryByTarget.get(state.target) === query) {
            this._currentQueryByTarget.delete(state.target);
        }
        _rawDeleteQuery(query._id);
    }
    isQuery(query) {
        if (!query || !query._id) return false;
        const state = this._queryRegistry.get(query._id);
        return !!(state && state.boundOnce && !state.deleted);
    }
    beginQuery(target, query) {
        if (!query || !query._id) return;
        const state = this._queryRegistry.get(query._id);
        if (!state || state.deleted) return;
        state.boundOnce = true;
        state.target = target;
        state.active = true;
        this._currentQueryByTarget.set(target, query);
        _rawBeginQuery(this._canvasId, target, query._id);
    }
    endQuery(target) {
        const query = this._currentQueryByTarget.get(target);
        if (query) {
            const state = this._queryRegistry.get(query._id);
            if (state) state.active = false;
        }
        this._currentQueryByTarget.delete(target);
        _rawEndQuery(this._canvasId, target);
    }
    getQuery(target, pname) {
        if (pname !== GL_CURRENT_QUERY) return null;
        return this._currentQueryByTarget.get(target) || null;
    }
    /**
     * Synchronous query parameter fetch.  Supported pname values:
     *   QUERY_RESULT           (0x8866) - u32 sample count / timer delta
     *   QUERY_RESULT_AVAILABLE (0x8867) - 0 or 1
     * Callers typically poll AVAILABLE before reading RESULT.
     */
    getQueryParameter(query, pname) {
        if (!query || !query._id) return 0;
        return _rawGetQueryParameter(query._id, pname);
    }

    // ---- Transform Feedback ------------------------------------
    createTransformFeedback() {
        // op_alloc_gl_resource_id: direct, no-submit.
        const id = op_alloc_gl_resource_id_webgl2();
        _rawCreateTransformFeedback(this._canvasId, id);
        const tf = { _id: id, _kind: 'tf' };
        this._tfRegistry.set(id, {
            active: false,
            boundOnce: false,
            deleted: false,
            paused: false,
        });
        return tf;
    }
    deleteTransformFeedback(tf) {
        if (!tf || !tf._id) return;
        const state = this._tfRegistry.get(tf._id);
        if (!state || state.deleted) return;
        if (state.active || state.paused) {
            this._pushJsError(GL_INVALID_OPERATION);
            return;
        }
        state.deleted = true;
        if (this._currentTransformFeedback === tf) {
            this._currentTransformFeedback = null;
        }
        _rawDeleteTransformFeedback(tf._id);
    }
    isTransformFeedback(tf) {
        if (!tf || !tf._id) return false;
        const state = this._tfRegistry.get(tf._id);
        return !!(state && state.boundOnce && !state.deleted);
    }
    bindTransformFeedback(target, tf) {
        this._currentTransformFeedback = tf || null;
        if (tf && tf._id) {
            const state = this._tfRegistry.get(tf._id);
            if (state && !state.deleted) {
                state.boundOnce = true;
            }
        }
        _rawBindTransformFeedback(this._canvasId, target, tf ? tf._id : 0);
    }
    beginTransformFeedback(primitiveMode) {
        if (this._currentTransformFeedback && this._currentTransformFeedback._id) {
            const state = this._tfRegistry.get(this._currentTransformFeedback._id);
            if (state && !state.deleted) {
                state.active = true;
                state.paused = false;
            }
        }
        _rawBeginTransformFeedback(this._canvasId, primitiveMode);
    }
    endTransformFeedback() {
        if (this._currentTransformFeedback && this._currentTransformFeedback._id) {
            const state = this._tfRegistry.get(this._currentTransformFeedback._id);
            if (state) {
                state.active = false;
                state.paused = false;
            }
        }
        _rawEndTransformFeedback(this._canvasId);
    }
    pauseTransformFeedback() {
        if (this._currentTransformFeedback && this._currentTransformFeedback._id) {
            const state = this._tfRegistry.get(this._currentTransformFeedback._id);
            if (state && state.active && !state.deleted) {
                state.paused = true;
            }
        }
        _rawPauseTransformFeedback(this._canvasId);
    }
    resumeTransformFeedback() {
        if (this._currentTransformFeedback && this._currentTransformFeedback._id) {
            const state = this._tfRegistry.get(this._currentTransformFeedback._id);
            if (state && state.active && !state.deleted) {
                state.paused = false;
            }
        }
        _rawResumeTransformFeedback(this._canvasId);
    }
    /**
     * transformFeedbackVaryings(program, varyings, bufferMode)
     * The varyings array is sent as a single US-separated string
     * because the fast op lane accepts one `#[string]` argument
     * per call.  ASCII 0x1F (Unit Separator) is chosen because
     * it can't legally appear in GLSL identifiers.
     */
    transformFeedbackVaryings(program, varyings, bufferMode) {
        if (!program || !program._id) return;
        const joined = (varyings || []).join('\x1f');
        _rawTransformFeedbackVaryings(this._canvasId, program._id, joined, bufferMode);
    }
    getTransformFeedbackVarying(program, index) {
        if (!program || !program._id) return null;
        const json = _rawGetTransformFeedbackVarying(program._id, index);
        if (!json) return null;
        try {
            return JSON.parse(json);
        } catch (_) {
            return null;
        }
    }

    // ---- 3D textures -------------------------------------------
    texImage3D(
        target, level, internalformat,
        width, height, depth, border,
        format, type, pixelsOrOffset, srcOffset
    ) {
        // WebGL 2 overloads: data may be ArrayBufferView + optional
        // srcOffset, a PBO offset integer, or null (reserve storage).
        let view = null;
        let bytesPerElement = 1;
        let elementOffset = Number(srcOffset) || 0;
        let pboOffset = -1;
        if (typeof pixelsOrOffset === 'number') {
            pboOffset = pixelsOrOffset | 0;
        } else if (pixelsOrOffset && pixelsOrOffset.buffer) {
            bytesPerElement = pixelsOrOffset.BYTES_PER_ELEMENT || 1;
            view = new Uint8Array(
                pixelsOrOffset.buffer,
                pixelsOrOffset.byteOffset || 0,
                pixelsOrOffset.byteLength || 0,
            );
        } else if (pixelsOrOffset instanceof ArrayBuffer) {
            view = new Uint8Array(pixelsOrOffset);
        }
        _rawTexImage3D(
            this._canvasId, target, level, internalformat,
            width, height, depth, border, format, type,
            view, elementOffset, bytesPerElement, pboOffset,
        );
    }
    texSubImage3D(
        target, level,
        xoffset, yoffset, zoffset,
        width, height, depth,
        format, type, pixelsOrOffset, srcOffset
    ) {
        let view = null;
        let bytesPerElement = 1;
        let elementOffset = Number(srcOffset) || 0;
        let pboOffset = -1;
        if (typeof pixelsOrOffset === 'number') {
            pboOffset = pixelsOrOffset | 0;
        } else if (pixelsOrOffset && pixelsOrOffset.buffer) {
            bytesPerElement = pixelsOrOffset.BYTES_PER_ELEMENT || 1;
            view = new Uint8Array(
                pixelsOrOffset.buffer,
                pixelsOrOffset.byteOffset || 0,
                pixelsOrOffset.byteLength || 0,
            );
        } else if (pixelsOrOffset) {
            view = new Uint8Array(pixelsOrOffset);
        }
        if (!view && pboOffset < 0) {
            return;
        }
        _rawTexSubImage3D(
            this._canvasId, target, level,
            xoffset, yoffset, zoffset,
            width, height, depth, format, type,
            view, elementOffset, bytesPerElement, pboOffset,
        );
    }
    texStorage3D(target, levels, internalformat, width, height, depth) {
        _rawTexStorage3D(
            this._canvasId, target, levels, internalformat,
            width, height, depth,
        );
    }
}

// GL batch flush is now handled by the unified frame-end hook in
// 02_2d_context.js (op_frame_end_unified). No separate GL hook needed.

export {
    WebGLRenderingContext,
    WebGL2RenderingContext,
};
