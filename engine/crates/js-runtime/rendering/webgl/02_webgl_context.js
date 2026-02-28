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
    op_enable_vertex_attrib_array,
    op_vertex_attrib_pointer,
    op_create_buffer,
    op_bind_buffer,
    op_buffer_data,
    op_get_uniform_location,
    op_uniform3f,
    op_uniform_matrix_3fv,
    op_gl_flush,
} from "ext:core/ops";

import { core, primordials } from "ext:core/mod.js";

const { isArrayBuffer, isTypedArray, isDataView } = core;

const {
    TypedArrayPrototypeGetBuffer,
    TypedArrayPrototypeGetByteLength,
    TypedArrayPrototypeGetByteOffset,
    Uint8Array,
    Uint32Array,
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
        throw new Error("drawingBufferWidth not supported");
    }

    get drawingBufferHeight() {
        throw new Error("drawingBufferHeight not supported");
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
        return new WebglObject(op_create_program());
    }

    useProgram(program) {
        return op_use_program(this._canvasId, program?.id);
    }

    linkProgram(program) {
        const programId = program?.id;
        const ok = op_link_program(programId);
        if (programId !== undefined) {
            // Linking can change active attrib/uniform locations and link status.
            this._invalidateProgramCaches(programId);
        }
        return ok;
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
        return new WebglObject(op_create_shader(type));
    }

    shaderSource(shader, src) {
        return op_shader_source(shader?.id, src);
    }

    compileShader(shader) {
        return op_compile_shader(shader?.id);
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
        return new WebglObject(op_create_buffer());
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
        if (id === null || id === undefined) {
            inner.set(name, null);
            return null;
        }
        const location = new WebglObject(id);
        inner.set(name, location);
        return location;
    }

    uniform3f(location, x, y, z) {
        op_uniform3f(this._canvasId, location?.id, x, y, z);
    }

    uniformMatrix3fv(location, transpose, value) {
        if (!(value instanceof Float32Array)) {
            throw new Error("Invalid data, must be a Float32Array");
        }
        op_uniform_matrix_3fv(this._canvasId, location?.id, transpose, toUnit32Array(value));
    }
}

Object.assign(WebGLRenderingContext.prototype, WebglConstants);

class WebGL2RenderingContext extends WebGLRenderingContext {
    constructor(canvas) {
        super(canvas);
    }

    bufferData(..._args) {
        throw new Error("Not implemented");
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
