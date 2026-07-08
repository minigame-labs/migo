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
        // Engines treat the canvas as a DOM element and set CSS on it (e.g.
        // Phaser's ScaleManager writes `canvas.style.marginLeft` to center it).
        // Absorb those writes on a plain object.
        this.style = {};

        registry.register(this, rid);
    }

    get width() {
        return this._width;
    }
    get height() {
        return this._height;
    }
    set width(v) {
        op_resize_canvas(this._rid, v, undefined);
        this._width = v;
        if (this._context && this._context._resetShadowState) {
            this._context._resetShadowState();
        }
    }
    set height(v) {
        op_resize_canvas(this._rid, undefined, v);
        this._height = v;
        if (this._context && this._context._resetShadowState) {
            this._context._resetShadowState();
        }
    }

    get clientWidth() {
        return this._width;
    }
    get clientHeight() {
        return this._height;
    }
    get offsetWidth() {
        return this._width;
    }
    get offsetHeight() {
        return this._height;
    }
    // Engines treat the canvas as a DOM element. Provide the minimal element
    // surface their scale/DOM managers touch (Phaser's ScaleManager measures
    // via getBoundingClientRect and reads/writes attributes; `style` is set in
    // the constructor). The canvas fills the surface at the origin.
    getBoundingClientRect() {
        const w = this._width, h = this._height;
        return { x: 0, y: 0, top: 0, left: 0, right: w, bottom: h, width: w, height: h };
    }
    setAttribute(name, value) {
        if (name === "width") this.width = value;
        else if (name === "height") this.height = value;
        else this["_attr_" + name] = value;
    }
    getAttribute(name) {
        if (name === "width") return this._width;
        if (name === "height") return this._height;
        const v = this["_attr_" + name];
        return v === undefined ? null : v;
    }
    removeAttribute(name) {
        delete this["_attr_" + name];
    }
    // Minimal EventTarget so engines can register canvas-level listeners
    // (notably `webglcontextlost` / `webglcontextrestored`; also touch). The
    // adapter's canvas shim only installs a no-op `addEventListener` when the
    // native canvas lacks one, so providing a real implementation here makes
    // engine listeners actually fire.
    addEventListener(type, listener) {
        if (typeof listener !== "function") return;
        if (!this._listeners) this._listeners = { __proto__: null };
        (this._listeners[type] || (this._listeners[type] = [])).push(listener);
    }
    removeEventListener(type, listener) {
        const arr = this._listeners && this._listeners[type];
        if (!arr) return;
        const i = arr.indexOf(listener);
        if (i !== -1) arr.splice(i, 1);
    }
    dispatchEvent(event) {
        const arr = this._listeners && this._listeners[event.type];
        if (arr) {
            // Copy so a listener removing itself mid-dispatch is safe.
            const copy = arr.slice();
            for (let i = 0; i < copy.length; i++) {
                try { copy[i].call(this, event); } catch (_e) { /* DOM swallows */ }
            }
        }
        // Also honor an `on<type>` property (e.g. canvas.onwebglcontextlost).
        const on = this["on" + event.type];
        if (typeof on === "function") {
            try { on.call(this, event); } catch (_e) { /* swallow */ }
        }
        return !event.defaultPrevented;
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

// Host-driven WebGL context-loss lifecycle. When the render thread rebuilds the
// GL share group after a real GPU reset (or a WEBGL_lose_context.loseContext
// simulation), it drives these events so the engine can drop and rebuild its
// own GL resources: `webglcontextlost` then, once the fresh context is ready,
// `webglcontextrestored`. Dispatched on the main (onscreen) canvas, which is
// where engines register their listeners. Invoked from the host via
// `_internalTriggerWebglContextEvent` (see 98_global_scope_window.js).
//
// NON-SPEC RECOVERY MODEL: unlike browsers, Migo recovery is MANDATORY and
// automatic -- the render thread always tears down and rebuilds the share group
// and only reports success after a probe. Recovery is therefore NOT gated on
// the listener calling `preventDefault()`. The event is still exposed as
// `cancelable` for engine compatibility, and `preventDefault()` is honored as a
// signal (returned to the host) rather than as the trigger for restoration.
// Engines should treat all GL resources as invalid on `webglcontextlost` and
// rebuild them on `webglcontextrestored`; `gl.isContextLost()` reflects the
// render-thread-owned authoritative state at all times.
//
// Returns `true` if a listener called `preventDefault()`, else `false`.
const dispatchWebglContextEvent = (type) => {
    if (!_mainCanvas) return false;
    let prevented = false;
    const event = {
        type,
        statusMessage: "",
        bubbles: false,
        cancelable: type === "webglcontextlost",
        get defaultPrevented() { return prevented; },
        preventDefault() { prevented = true; },
    };
    _mainCanvas.dispatchEvent(event);
    return prevented;
};

export {
    createCanvas,
    createOffscreenCanvas,
    getMainCanvas,
    dispatchWebglContextEvent,
};
