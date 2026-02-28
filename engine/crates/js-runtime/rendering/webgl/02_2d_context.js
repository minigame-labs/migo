/**
 * Canvas 2D Context - Command Batching Implementation
 *
 * All draw commands within a RAF frame are batched and sent as a single
 * message to the render thread, significantly reducing IPC overhead.
 */

import {
    op_create_context_2d,
    op_measure_text,
    op_get_image_data,
    // Frame lifecycle
    op_frame_begin,
    op_frame_end,
    op_frame_end_all,
    // Path methods
    op_begin_path,
    op_close_path,
    op_move_to,
    op_line_to,
    op_quadratic_curve_to,
    op_bezier_curve_to,
    op_arc,
    op_arc_to,
    op_rect,
    op_ellipse,
    // Drawing methods
    op_fill,
    op_stroke,
    op_clip,
    // Rectangle methods
    op_fill_rect,
    op_stroke_rect,
    op_clear_rect,
    // Text methods
    op_fill_text,
    op_stroke_text,
    // Style setters
    op_set_fill_style,
    op_set_stroke_style,
    op_set_line_width,
    op_set_line_cap,
    op_set_line_join,
    op_set_miter_limit,
    op_set_global_alpha,
    op_set_font,
    op_set_text_align,
    op_set_text_baseline,
    // State methods
    op_save,
    op_restore,
    // Transform methods
    op_translate,
    op_rotate,
    op_scale,
    op_set_transform,
    op_reset_transform,
    // Image methods
    op_draw_image,
    op_draw_image_batch,
} from "ext:core/ops";

// Line cap constants
const LINE_CAP_MAP = { 'butt': 0, 'round': 1, 'square': 2 };
// Line join constants
const LINE_JOIN_MAP = { 'miter': 0, 'round': 1, 'bevel': 2 };
// Text align constants
const TEXT_ALIGN_MAP = { 'start': 0, 'end': 1, 'left': 2, 'right': 3, 'center': 4 };
// Text baseline constants
const TEXT_BASELINE_MAP = {
    'top': 0, 'hanging': 1, 'middle': 2,
    'alphabetic': 3, 'ideographic': 4, 'bottom': 5,
};

class CanvasRenderingContext2D {
    constructor(canvas) {
        this._canvas = canvas;
        this._canvasId = canvas._rid;

        // Create native 2D context on the render thread
        this._ctxId = op_create_context_2d(this._canvasId);
        if (this._ctxId < 0) { console.error("Failed to create 2d context"); }

        // Shadow state (for JS-side queries)
        this._fillStyle = '#000000';
        this._strokeStyle = '#000000';
        this._lineWidth = 1;
        this._lineCap = 'butt';
        this._lineJoin = 'miter';
        this._miterLimit = 10;
        this._globalAlpha = 1;
        this._font = '10px sans-serif';
        this._textAlign = 'start';
        this._textBaseline = 'alphabetic';

        // State stack for save/restore
        this._stateStack = [];

        // Frame tracking
        this._frameStarted = false;
    }

    get canvas() { return this._canvas; }

    // ==================== Frame Lifecycle ====================

    _frameBegin() {
        if (!this._frameStarted) {
            op_frame_begin(this._canvasId);
            this._frameStarted = true;
        }
    }

    _frameEnd() {
        if (this._frameStarted) {
            op_frame_end(this._canvasId);
            this._frameStarted = false;
        }
    }

    // ==================== Path Methods ====================

    beginPath() {
        this._frameBegin();
        op_begin_path(this._canvasId);
    }

    closePath() {
        op_close_path(this._canvasId);
    }

    moveTo(x, y) {
        op_move_to(this._canvasId, x, y);
    }

    lineTo(x, y) {
        op_line_to(this._canvasId, x, y);
    }

    quadraticCurveTo(cpx, cpy, x, y) {
        op_quadratic_curve_to(this._canvasId, cpx, cpy, x, y);
    }

    bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x, y) {
        op_bezier_curve_to(this._canvasId, cp1x, cp1y, cp2x, cp2y, x, y);
    }

    arc(x, y, radius, startAngle, endAngle, counterclockwise = false) {
        op_arc(this._canvasId, x, y, radius, startAngle, endAngle, counterclockwise);
    }

    arcTo(x1, y1, x2, y2, radius) {
        op_arc_to(this._canvasId, x1, y1, x2, y2, radius);
    }

    rect(x, y, width, height) {
        op_rect(this._canvasId, x, y, width, height);
    }

    ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle, counterclockwise = false) {
        op_ellipse(this._canvasId, x, y, radiusX, radiusY, rotation, startAngle, endAngle, counterclockwise);
    }

    // ==================== Drawing Methods ====================

    fill(pathOrFillRule) {
        op_fill(this._canvasId);
    }

    stroke(path) {
        op_stroke(this._canvasId);
    }

    clip(pathOrFillRule) {
        op_clip(this._canvasId);
    }

    // ==================== Rectangle Methods ====================

    fillRect(x, y, width, height) {
        this._frameBegin();
        op_fill_rect(this._canvasId, x, y, width, height);
    }

    strokeRect(x, y, width, height) {
        this._frameBegin();
        op_stroke_rect(this._canvasId, x, y, width, height);
    }

    clearRect(x, y, width, height) {
        this._frameBegin();
        op_clear_rect(this._canvasId, x, y, width, height);
    }

    // ==================== Text Methods ====================

    fillText(text, x, y, maxWidth = Infinity) {
        this._frameBegin();
        op_fill_text(this._canvasId, String(text), x, y, maxWidth);
    }

    strokeText(text, x, y, maxWidth = Infinity) {
        this._frameBegin();
        op_stroke_text(this._canvasId, String(text), x, y, maxWidth);
    }

    measureText(text) {
        return op_measure_text(this._canvasId, String(text));
    }

    // ==================== Style Properties ====================

    get fillStyle() { return this._fillStyle; }
    set fillStyle(value) {
        this._fillStyle = value;
        this._frameBegin();
        op_set_fill_style(this._canvasId, String(value));
    }

    get strokeStyle() { return this._strokeStyle; }
    set strokeStyle(value) {
        this._strokeStyle = value;
        this._frameBegin();
        op_set_stroke_style(this._canvasId, String(value));
    }

    get lineWidth() { return this._lineWidth; }
    set lineWidth(value) {
        this._lineWidth = value;
        this._frameBegin();
        op_set_line_width(this._canvasId, value);
    }

    get lineCap() { return this._lineCap; }
    set lineCap(value) {
        this._lineCap = value;
        this._frameBegin();
        op_set_line_cap(this._canvasId, LINE_CAP_MAP[value] ?? 0);
    }

    get lineJoin() { return this._lineJoin; }
    set lineJoin(value) {
        this._lineJoin = value;
        this._frameBegin();
        op_set_line_join(this._canvasId, LINE_JOIN_MAP[value] ?? 0);
    }

    get miterLimit() { return this._miterLimit; }
    set miterLimit(value) {
        this._miterLimit = value;
        this._frameBegin();
        op_set_miter_limit(this._canvasId, value);
    }

    get globalAlpha() { return this._globalAlpha; }
    set globalAlpha(value) {
        this._globalAlpha = Math.max(0, Math.min(1, value));
        this._frameBegin();
        op_set_global_alpha(this._canvasId, this._globalAlpha);
    }

    get font() { return this._font; }
    set font(value) {
        this._font = value;
        this._frameBegin();
        op_set_font(this._canvasId, value);
    }

    get textAlign() { return this._textAlign; }
    set textAlign(value) {
        this._textAlign = value;
        this._frameBegin();
        op_set_text_align(this._canvasId, TEXT_ALIGN_MAP[value] ?? 0);
    }

    get textBaseline() { return this._textBaseline; }
    set textBaseline(value) {
        this._textBaseline = value;
        this._frameBegin();
        op_set_text_baseline(this._canvasId, TEXT_BASELINE_MAP[value] ?? 3);
    }

    // ==================== State Methods ====================

    save() {
        this._stateStack.push({
            fillStyle: this._fillStyle,
            strokeStyle: this._strokeStyle,
            lineWidth: this._lineWidth,
            lineCap: this._lineCap,
            lineJoin: this._lineJoin,
            miterLimit: this._miterLimit,
            globalAlpha: this._globalAlpha,
            font: this._font,
            textAlign: this._textAlign,
            textBaseline: this._textBaseline,
        });
        this._frameBegin();
        op_save(this._canvasId);
    }

    restore() {
        if (this._stateStack.length > 0) {
            const state = this._stateStack.pop();
            Object.assign(this, {
                _fillStyle: state.fillStyle,
                _strokeStyle: state.strokeStyle,
                _lineWidth: state.lineWidth,
                _lineCap: state.lineCap,
                _lineJoin: state.lineJoin,
                _miterLimit: state.miterLimit,
                _globalAlpha: state.globalAlpha,
                _font: state.font,
                _textAlign: state.textAlign,
                _textBaseline: state.textBaseline,
            });
        }
        this._frameBegin();
        op_restore(this._canvasId);
    }

    // ==================== Transform Methods ====================

    translate(x, y) {
        this._frameBegin();
        op_translate(this._canvasId, x, y);
    }

    rotate(angle) {
        this._frameBegin();
        op_rotate(this._canvasId, angle);
    }

    scale(x, y) {
        this._frameBegin();
        op_scale(this._canvasId, x, y);
    }

    setTransform(a, b, c, d, e, f) {
        this._frameBegin();
        op_set_transform(this._canvasId, a, b, c, d, e, f);
    }

    resetTransform() {
        this._frameBegin();
        op_reset_transform(this._canvasId);
    }

    transform(a, b, c, d, e, f) {
        // transform() multiplies current matrix; approximate with setTransform
        this._frameBegin();
        op_set_transform(this._canvasId, a, b, c, d, e, f);
    }

    getTransform() {
        return { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
    }

    // ==================== Image Methods ====================

    drawImage(image, ...args) {
        if (!image || !image.loaded) return;

        this._frameBegin();

        let sx, sy, sw, sh, dx, dy, dw, dh;

        if (args.length === 2) {
            [dx, dy] = args;
            sx = sy = 0;
            sw = image.width;
            sh = image.height;
            dw = sw;
            dh = sh;
        } else if (args.length === 4) {
            [dx, dy, dw, dh] = args;
            sx = sy = 0;
            sw = image.width;
            sh = image.height;
        } else if (args.length === 8) {
            [sx, sy, sw, sh, dx, dy, dw, dh] = args;
        } else {
            return;
        }

        op_draw_image(this._canvasId, image.rid, sx, sy, sw, sh, dx, dy, dw, dh);
    }

    drawImageBatch(draws) {
        if (!Array.isArray(draws) || draws.length === 0) return;

        this._frameBegin();

        const validDraws = draws.filter(d => d.image && d.image.loaded);
        if (validDraws.length === 0) return;

        const buffer = new Float32Array(validDraws.length * 9);
        let offset = 0;

        for (const d of validDraws) {
            buffer[offset++] = d.image.rid;
            buffer[offset++] = d.sx ?? -1;
            buffer[offset++] = d.sy ?? -1;
            buffer[offset++] = d.sw ?? -1;
            buffer[offset++] = d.sh ?? -1;
            buffer[offset++] = d.dx;
            buffer[offset++] = d.dy;
            buffer[offset++] = d.dw ?? -1;
            buffer[offset++] = d.dh ?? -1;
        }

        op_draw_image_batch(this._canvasId, new Uint8Array(buffer.buffer));
    }

    getImageData(sx, sy, sw, sh) {
        const data = op_get_image_data(this._canvasId, sx, sy, sw, sh);
        return { width: sw, height: sh, data: new Uint8ClampedArray(data) };
    }

    createImageData(sw, sh) {
        return { width: sw, height: sh, data: new Uint8ClampedArray(sw * sh * 4) };
    }

    putImageData(imageData, dx, dy) {
        // Not implemented
    }

    // ==================== Compositing (stubs) ====================
    get globalCompositeOperation() { return "source-over"; }
    set globalCompositeOperation(value) { }

    // ==================== Shadows (stubs) ====================
    get shadowBlur() { return 0; }
    set shadowBlur(value) { }
    get shadowColor() { return "rgba(0,0,0,0)"; }
    set shadowColor(value) { }
    get shadowOffsetX() { return 0; }
    set shadowOffsetX(value) { }
    get shadowOffsetY() { return 0; }
    set shadowOffsetY(value) { }

    // ==================== Other stubs ====================
    isPointInPath() { return false; }
    isPointInStroke() { return false; }
    createLinearGradient() { return {}; }
    createRadialGradient() { return {}; }
    createPattern() { return {}; }
    setLineDash() { }
    getLineDash() { return []; }
    get lineDashOffset() { return 0; }
    set lineDashOffset(value) { }
}

// Frame-end callback registry.  Each module registers its own cleanup
// function instead of overwriting a global.  This avoids load-order
// dependencies between 2D and WebGL modules.
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
    op_frame_end_all();
});

export { CanvasRenderingContext2D };
