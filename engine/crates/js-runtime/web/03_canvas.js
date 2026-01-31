import { primordials } from "ext:core/mod.js";
import { op_create_canvas, op_get_canvas_info, op_resize_canvas, op_destroy_canvas } from "ext:core/ops";
import { WebGLRenderingContext, WebGL2RenderingContext } from "ext:host_v8_webgl/02_webgl_context.js";
import { CanvasRenderingContext2D } from "ext:host_v8_webgl/02_2d_context.js";
const { SafeFinalizationRegistry } = primordials;

const registry = new SafeFinalizationRegistry((rid) => {
    op_destroy_canvas(rid);
});

class Canvas {
    constructor() {
        this._rid = op_create_canvas(1, 1);
        if (this._rid < 0) {
            console.error("Failed to create canvas");
            this._rid = 1; // fallback to main canvas
        }
        this._offscreen = this._rid !== 1;
        const info = op_get_canvas_info(this._rid);
        this._width = info['0'];
        this._height = info['1'];

        registry.register(this, this._rid);
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

const createCanvas = () => {
    return new Canvas();
};

export {
    createCanvas,
};