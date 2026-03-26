import {
    op_viewport,
    op_clear,
    op_clear_color,
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
    op_gl_flush,
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
    op_bind_renderbuffer,
    op_renderbuffer_storage,
    op_read_pixels,
    op_hint,
} from "ext:core/ops";

import { core, primordials } from "ext:core/mod.js";

const { isArrayBuffer, isTypedArray, isDataView } = core;

const {
    TypedArrayPrototypeGetBuffer,
    TypedArrayPrototypeGetByteLength,
    TypedArrayPrototypeGetByteOffset,
    Uint8Array,
    Uint32Array,
    Int32Array,
    Float32Array,
    DataViewPrototypeGetBuffer,
    DataViewPrototypeGetByteLength,
    DataViewPrototypeGetByteOffset,
    ArrayBufferPrototypeGetByteLength,
} = primordials;

import { WebglConstants } from "./01_constants.js";

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
    }
    throw new TypeError("Invalid input: must be a TypedArray, DataView, or ArrayBuffer");
}

function toUnit8Array(input) {
    return toTypedArray(input, Uint8Array);
}

function toUnit32Array(input) {
    return toTypedArray(input, Uint32Array);
}

// Reinterpret Int32Array bits as Uint32Array without copying.
// Needed because deno_core #[buffer(copy)] only accepts Vec<u32>;
// Rust cast_vec then reinterprets the bits back to the target type.
function toInt32AsUint32(input) {
    const i32 = toTypedArray(input, Int32Array);
    return new Uint32Array(i32.buffer, i32.byteOffset, i32.length);
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
        this._options = options;
        this._canvasId = canvas._rid;
        // Client-side monotonic ID counter for GL resources.
        // IDs are assigned locally and sent to the render thread as
        // fire-and-forget, eliminating sync round-trips on create*.
        this._nextResourceId = 1;
        // Nested Map: programId -> Map(name -> location)
        // Allows O(1) per-program invalidation via .delete(programId).
        this._attribLocationCache = new Map();
        this._uniformLocationCache = new Map();
        this._programParameterCache = new Map();
        // shaderId -> Map(pname -> value)
        this._shaderParameterCache = new Map();
    }

    /** Invalidate all cached locations/params for a given program. O(1). */
    _invalidateProgramCaches(programId) {
        this._attribLocationCache.delete(programId);
        this._uniformLocationCache.delete(programId);
        this._programParameterCache.delete(programId);
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
        op_viewport(this._canvasId, x, y, width, height);
    }

    clearColor(r, g, b, a) {
        return op_clear_color(this._canvasId, r, g, b, a);
    }

    clear(mask) {
        return op_clear(this._canvasId, mask);
    }

    createProgram() {
        const id = this._nextResourceId++;
        op_create_program(id);
        return new WebglObject(id);
    }

    useProgram(program) {
        return op_use_program(this._canvasId, program?.id);
    }

    linkProgram(program) {
        const programId = program?.id;
        op_link_program(programId);
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
        const param = op_get_program_parameter(programId, pname);
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
        return op_get_program_info_log(program?.id);
    }

    deleteProgram(program) {
        const programId = program?.id;
        op_delete_program(programId);
        if (programId !== undefined) {
            this._invalidateProgramCaches(programId);
        }
    }

    createShader(type) {
        const id = this._nextResourceId++;
        op_create_shader(id, type);
        return new WebglObject(id);
    }

    shaderSource(shader, src) {
        return op_shader_source(shader?.id, src);
    }

    compileShader(shader) {
        op_compile_shader(shader?.id);
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
            const val = op_get_shader_parameter(shaderId, pname);
            inner.set(pname, val);
            return val;
        }
        const ret = op_get_shader_parameter(shaderId, pname);
        if (pname === WebglConstants.COMPILE_STATUS || pname === WebglConstants.DELETE_STATUS) {
            return Boolean(ret);
        }
        return ret;
    }

    attachShader(program, shader) {
        return op_attach_shader(program?.id, shader?.id);
    }

    getShaderInfoLog(shader) {
        return op_get_shader_info_log(shader?.id);
    }

    deleteShader(shader) {
        const shaderId = shader?.id;
        op_delete_shader(shaderId);
        if (shaderId !== undefined) {
            this._shaderParameterCache.delete(shaderId);
        }
    }

    drawArrays(mode, first, count) {
        return op_draw_arrays(this._canvasId, mode, first, count);
    }

    drawElements(mode, count, type, offset) {
        return op_draw_elements(this._canvasId, mode, count, type, offset);
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
        const location = op_get_attrib_location(this._canvasId, programId, name);
        inner.set(name, location);
        return location;
    }

    getActiveAttrib(program, index) {
        const programId = program?.id;
        if (programId === undefined) return null;
        const json = op_get_active_attrib(this._canvasId, programId, index >>> 0);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    getActiveUniform(program, index) {
        const programId = program?.id;
        if (programId === undefined) return null;
        const json = op_get_active_uniform(this._canvasId, programId, index >>> 0);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    enableVertexAttribArray(index) {
        return op_enable_vertex_attrib_array(this._canvasId, index);
    }

    vertexAttribPointer(index, size, type, normalized, stride, offset) {
        return op_vertex_attrib_pointer(
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
        const id = this._nextResourceId++;
        op_create_buffer(id);
        return new WebglObject(id);
    }

    bindBuffer(target, buffer) {
        // use -1 to indicate unbinding
        return op_bind_buffer(this._canvasId, target, buffer?.id || -1);
    }

    bufferData(target, srcOrSize, usage) {
        if (typeof srcOrSize === "number") {
            const size = srcOrSize >>> 0;
            return op_buffer_data(this._canvasId, target, size, null, usage);
        } else {
            const u8 = toUnit8Array(srcOrSize);
            return op_buffer_data(this._canvasId, target, -1, u8, usage);
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
        const id = op_get_uniform_location(this._canvasId, programId, name);
        if (id < 0) {
            inner.set(name, null);
            return null;
        }
        const location = new WebglObject(id);
        inner.set(name, location);
        return location;
    }

    uniform3f(location, x, y, z) {
        op_uniform3f(this._canvasId, _loc(location), x, y, z);
    }

    uniformMatrix3fv(location, transpose, value) {
        if (!(value instanceof Float32Array)) {
            throw new Error("Invalid data, must be a Float32Array");
        }
        op_uniform_matrix_3fv(this._canvasId, _loc(location), transpose, toUnit32Array(value));
    }

    // -- Phase 1A: GL State --

    enable(cap) {
        op_enable(this._canvasId, cap);
    }

    disable(cap) {
        op_disable(this._canvasId, cap);
    }

    isEnabled(cap) {
        return Boolean(op_is_enabled(this._canvasId, cap));
    }

    getParameter(pname) {
        const json = op_get_parameter(this._canvasId, pname);
        if (!json) return null;
        try { return JSON.parse(json); } catch (_) { return null; }
    }

    getError() {
        return 0;
    }

    getExtension(_name) {
        return null;
    }

    // -- Phase 1B: Textures --

    createTexture() {
        const id = this._nextResourceId++;
        op_create_texture(id);
        return new WebglObject(id);
    }

    deleteTexture(texture) {
        if (texture && texture.id !== undefined) op_delete_texture(texture.id);
    }

    bindTexture(target, texture) {
        op_bind_texture(this._canvasId, target, texture ? texture.id : -1);
    }

    activeTexture(unit) {
        op_active_texture(this._canvasId, unit);
    }

    texImage2D(target, level, internalformat, a4, a5, a6, a7, a8, a9) {
        // 9-arg: (target, level, internalformat, width, height, border, format, type, pixels)
        // 6-arg: (target, level, internalformat, format, type, source)
        if (a7 !== undefined) {
            const data = a9 != null ? toUnit8Array(a9) : null;
            op_tex_image_2d(this._canvasId, target, level, internalformat, a4, a5, a6, a7, a8, data);
        } else {
            const source = a6;
            const imageId = source && typeof source.rid === "number" ? source.rid : null;
            if (imageId != null) {
                op_tex_image_2d_from_image(this._canvasId, target, level, internalformat, a4, a5, imageId);
            } else {
                const raw = sourceToRawRgba(source);
                if (raw) {
                    op_tex_image_2d(
                        this._canvasId,
                        target,
                        level,
                        internalformat,
                        raw.width,
                        raw.height,
                        0,
                        a4,
                        a5,
                        raw.data,
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
            const data = toUnit8Array(pixels);
            op_tex_sub_image_2d(this._canvasId, target, level, xoffset, yoffset, width, height, format, type, data);
            return;
        }

        // 7-arg: (..., format, type, source)
        const source = format;
        const sourceFormat = width;
        const sourceType = height;
        const imageId = source && typeof source.rid === "number" ? source.rid : null;
        if (imageId != null) {
            op_tex_sub_image_2d_from_image(
                this._canvasId,
                target,
                level,
                xoffset,
                yoffset,
                sourceFormat,
                sourceType,
                imageId,
            );
        } else {
            const raw = sourceToRawRgba(source);
            if (raw) {
                op_tex_sub_image_2d(
                    this._canvasId,
                    target,
                    level,
                    xoffset,
                    yoffset,
                    raw.width,
                    raw.height,
                    sourceFormat,
                    sourceType,
                    raw.data,
                );
                return;
            }
            const kind = source && source.constructor ? source.constructor.name : typeof source;
            console.warn(`texSubImage2D 7-argument form unsupported source: ${kind}`);
        }
    }

    texParameteri(target, pname, param) {
        op_tex_parameteri(this._canvasId, target, pname, param);
    }

    texParameterf(target, pname, param) {
        op_tex_parameterf(this._canvasId, target, pname, param);
    }

    generateMipmap(target) {
        op_generate_mipmap(this._canvasId, target);
    }

    pixelStorei(pname, param) {
        let value;
        if (param === true) value = 1;
        else if (param === false) value = 0;
        else value = Number(param) | 0;
        op_pixel_storei(this._canvasId, pname, value);
    }

    compressedTexImage2D(target, level, internalformat, width, height, border, data) {
        const u8 = toUnit8Array(data);
        op_compressed_tex_image_2d(this._canvasId, target, level, internalformat, width, height, border, u8);
    }

    compressedTexSubImage2D(target, level, xoffset, yoffset, width, height, format, data) {
        const u8 = toUnit8Array(data);
        op_compressed_tex_sub_image_2d(this._canvasId, target, level, xoffset, yoffset, width, height, format, u8);
    }

    // -- Phase 1C: Buffer & Vertex Extensions --

    bufferSubData(target, offset, data) {
        const u8 = toUnit8Array(data);
        op_buffer_sub_data(this._canvasId, target, offset, u8);
    }

    disableVertexAttribArray(index) {
        op_disable_vertex_attrib_array(this._canvasId, index);
    }

    clearDepth(depth) {
        op_clear_depth(this._canvasId, depth);
    }

    clearStencil(s) {
        op_clear_stencil(this._canvasId, s);
    }

    // -- Phase 2A: Blend/Depth/Stencil/Cull State --

    blendFunc(sfactor, dfactor) { op_blend_func(this._canvasId, sfactor, dfactor); }
    blendFuncSeparate(srcRGB, dstRGB, srcAlpha, dstAlpha) { op_blend_func_separate(this._canvasId, srcRGB, dstRGB, srcAlpha, dstAlpha); }
    blendEquation(mode) { op_blend_equation(this._canvasId, mode); }
    blendEquationSeparate(modeRGB, modeAlpha) { op_blend_equation_separate(this._canvasId, modeRGB, modeAlpha); }
    blendColor(r, g, b, a) { op_blend_color(this._canvasId, r, g, b, a); }
    depthFunc(func) { op_depth_func(this._canvasId, func); }
    depthMask(flag) { op_depth_mask(this._canvasId, flag); }
    depthRange(near, far) { op_depth_range(this._canvasId, near, far); }
    stencilFunc(func, ref_, mask) { op_stencil_func(this._canvasId, func, ref_, mask); }
    stencilFuncSeparate(face, func, ref_, mask) { op_stencil_func_separate(this._canvasId, face, func, ref_, mask); }
    stencilOp(fail, zfail, zpass) { op_stencil_op(this._canvasId, fail, zfail, zpass); }
    stencilOpSeparate(face, fail, zfail, zpass) { op_stencil_op_separate(this._canvasId, face, fail, zfail, zpass); }
    stencilMask(mask) { op_stencil_mask(this._canvasId, mask); }
    stencilMaskSeparate(face, mask) { op_stencil_mask_separate(this._canvasId, face, mask); }
    cullFace(mode) { op_cull_face(this._canvasId, mode); }
    frontFace(mode) { op_front_face(this._canvasId, mode); }
    colorMask(r, g, b, a) { op_color_mask(this._canvasId, r, g, b, a); }
    scissor(x, y, width, height) { op_scissor(this._canvasId, x, y, width, height); }
    lineWidth(width) { op_line_width(this._canvasId, width); }
    polygonOffset(factor, units) { op_polygon_offset(this._canvasId, factor, units); }

    // -- Phase 2B: Uniform Variants --

    uniform1i(location, x) {
        op_uniform1i(this._canvasId, _loc(location), x);
    }
    uniform1f(location, x) {
        op_uniform1f(this._canvasId, _loc(location), x);
    }
    uniform2f(location, x, y) {
        op_uniform2f(this._canvasId, _loc(location), x, y);
    }
    uniform4f(location, x, y, z, w) {
        op_uniform4f(this._canvasId, _loc(location), x, y, z, w);
    }
    uniform1iv(location, value) {
        op_uniform1iv(this._canvasId, _loc(location), toInt32AsUint32(value));
    }
    uniform1fv(location, value) {
        op_uniform1fv(this._canvasId, _loc(location), toUnit32Array(value));
    }
    uniform2iv(location, value) {
        op_uniform2iv(this._canvasId, _loc(location), toInt32AsUint32(value));
    }
    uniform2fv(location, value) {
        op_uniform2fv(this._canvasId, _loc(location), toUnit32Array(value));
    }
    uniform3iv(location, value) {
        op_uniform3iv(this._canvasId, _loc(location), toInt32AsUint32(value));
    }
    uniform3fv(location, value) {
        op_uniform3fv(this._canvasId, _loc(location), toUnit32Array(value));
    }
    uniform4iv(location, value) {
        op_uniform4iv(this._canvasId, _loc(location), toInt32AsUint32(value));
    }
    uniform4fv(location, value) {
        op_uniform4fv(this._canvasId, _loc(location), toUnit32Array(value));
    }
    uniformMatrix2fv(location, transpose, value) {
        op_uniform_matrix_2fv(this._canvasId, _loc(location), transpose, toUnit32Array(value));
    }
    uniformMatrix4fv(location, transpose, value) {
        op_uniform_matrix_4fv(this._canvasId, _loc(location), transpose, toUnit32Array(value));
    }

    // -- Phase 3A: Framebuffer/Renderbuffer --

    createFramebuffer() {
        const id = this._nextResourceId++;
        op_create_framebuffer(id);
        return new WebglObject(id);
    }
    deleteFramebuffer(fb) {
        if (fb && fb.id !== undefined) op_delete_framebuffer(fb.id);
    }
    bindFramebuffer(target, fb) {
        op_bind_framebuffer(this._canvasId, target, fb ? fb.id : -1);
    }
    framebufferTexture2D(target, attachment, textarget, texture, level) {
        op_framebuffer_texture_2d(this._canvasId, target, attachment, textarget, texture ? texture.id : -1, level);
    }
    framebufferRenderbuffer(target, attachment, renderbuffertarget, renderbuffer) {
        op_framebuffer_renderbuffer(this._canvasId, target, attachment, renderbuffertarget, renderbuffer ? renderbuffer.id : -1);
    }
    checkFramebufferStatus(target) {
        return op_check_framebuffer_status(this._canvasId, target);
    }
    createRenderbuffer() {
        const id = this._nextResourceId++;
        op_create_renderbuffer(id);
        return new WebglObject(id);
    }
    deleteRenderbuffer(rb) {
        if (rb && rb.id !== undefined) op_delete_renderbuffer(rb.id);
    }
    bindRenderbuffer(target, rb) {
        op_bind_renderbuffer(this._canvasId, target, rb ? rb.id : -1);
    }
    renderbufferStorage(target, internalformat, width, height) {
        op_renderbuffer_storage(this._canvasId, target, internalformat, width, height);
    }

    // -- Phase 3B: Misc --

    readPixels(x, y, width, height, format, type, pixels) {
        const data = op_read_pixels(this._canvasId, x, y, width, height, format, type);
        if (data && pixels) {
            const u8 = new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength);
            u8.set(data.subarray(0, u8.length));
        }
    }
    hint(target, mode) {
        op_hint(this._canvasId, target, mode);
    }
}

Object.assign(WebGLRenderingContext.prototype, WebglConstants);

class WebGL2RenderingContext extends WebGLRenderingContext {
    constructor(canvas) {
        super(canvas);
    }
}

// Register GL batch flush into the frame-end hook registry.
// Uses the same __migo_frame_end_hooks array as 02_2d_context.js,
// making the system load-order independent.
if (!globalThis.__migo_frame_end_hooks) {
    globalThis.__migo_frame_end_hooks = [];
    globalThis.__migo_frame_end_all = () => {
        const hooks = globalThis.__migo_frame_end_hooks;
        for (let i = 0; i < hooks.length; i++) {
            hooks[i]();
        }
    };
}
globalThis.__migo_frame_end_hooks.push(() => {
    op_gl_flush();
});

export {
    WebGLRenderingContext,
    WebGL2RenderingContext,
};
