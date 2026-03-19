import { primordials } from "ext:core/mod.js";
import { op_create_offscreen_canvas, op_get_canvas_info, op_resize_canvas, op_destroy_canvas } from "ext:core/ops";
import { WebGLRenderingContext, WebGL2RenderingContext } from "ext:host_v8_webgl/02_webgl_context.js";
import { CanvasRenderingContext2D } from "ext:host_v8_webgl/02_2d_context.js";
const { SafeFinalizationRegistry } = primordials;

const registry = new SafeFinalizationRegistry((rid) => {
    op_destroy_canvas(rid);
});

let _mainCanvas = null;
let _isFirstCreate = true;

class Canvas {
    constructor(rid) {
        this._rid = rid;
        this._offscreen = rid !== 1;
        const info = op_get_canvas_info(rid);
        this._width = info['0'];
        this._height = info['1'];

        registry.register(this, rid);
    }

    get width() {
        return op_get_canvas_info(this._rid)[0];
    }
    get height() {
        return op_get_canvas_info(this._rid)[1];
    }
    set width(v) {
        op_resize_canvas(this._rid, v, undefined);
        this._width = v;
    }
    set height(v) {
        op_resize_canvas(this._rid, undefined, v);
        this._height = v;
    }

    get clientWidth() {
        return op_get_canvas_info(this._rid)[0];
    }
    get clientHeight() {
        return op_get_canvas_info(this._rid)[1];
    }

    getContext(contextType, options) {
        if (this._context) { return this._context; }
        if (contextType === 'webgl2') {
            this._context = new WebGL2RenderingContext(this, options);
        } else if (contextType === 'webgl' || contextType === 'experimental-webgl') {
            this._context = new WebGLRenderingContext(this, options);
        } else {
            this._context = new CanvasRenderingContext2D(this);
        }
        return this._context;
    }
}

// Game API: first call returns the main canvas, subsequent calls return offscreen.
const createCanvas = () => {
    if (_isFirstCreate) {
        _isFirstCreate = false;
        return getMainCanvas();
    }
    return createOffscreenCanvas(1, 1);
};

// SDK internal: always creates an offscreen canvas, never touches rid 1.
const createOffscreenCanvas = (width, height) => {
    const rid = op_create_offscreen_canvas(width || 1, height || 1);
    return new Canvas(rid);
};

// Returns the main canvas (rid 1). Lazily wraps rid 1 on first access.
const getMainCanvas = () => {
    if (!_mainCanvas) {
        _mainCanvas = new Canvas(1);
    }
    return _mainCanvas;
};

export {
    createCanvas,
    createOffscreenCanvas,
    getMainCanvas,
};
