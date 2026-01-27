import {
    op_create_context_2d,
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
    op_measure_text,
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
    op_get_image_data,
} from "ext:core/ops";

// Text align: 0=start, 1=end, 2=left, 3=right, 4=center
const TEXT_ALIGN_MAP = { start: 0, end: 1, left: 2, right: 3, center: 4 };
// Text baseline: 0=top, 1=hanging, 2=middle, 3=alphabetic, 4=ideographic, 5=bottom
const TEXT_BASELINE_MAP = { top: 0, hanging: 1, middle: 2, alphabetic: 3, ideographic: 4, bottom: 5 };
// Line cap: 0=butt, 1=round, 2=square
const LINE_CAP_MAP = { butt: 0, round: 1, square: 2 };
// Line join: 0=miter, 1=round, 2=bevel
const LINE_JOIN_MAP = { miter: 0, round: 1, bevel: 2 };

class CanvasRenderingContext2D {
    constructor(canvas) {
        this._canvas = canvas;
        this._canvasId = canvas._rid;
        this._ctxId = op_create_context_2d(this._canvasId);
        if (this._ctxId < 0) { console.error("Failed to create 2d context"); }
        // Initialize state
        this._fillStyle = "rgb(0,0,0)";
        this._strokeStyle = "rgb(0,0,0)";
        this._lineWidth = 1;
        this._lineCap = "butt";
        this._lineJoin = "miter";
        this._miterLimit = 10;
        this._globalAlpha = 1.0;
        this._font = "10px sans-serif";
        this._textAlign = "start";
        this._textBaseline = "alphabetic";
    }

    get canvas() { return this._canvas; }

    // ========== Style properties ==========
    get fillStyle() { return this._fillStyle; }
    set fillStyle(value) {
        if (this._fillStyle === value) return;
        if (typeof value !== "string") { throw new TypeError("fillStyle must be a string"); }
        op_set_fill_style(this._canvasId, value);
        this._fillStyle = value;
    }

    get strokeStyle() { return this._strokeStyle; }
    set strokeStyle(value) {
        if (this._strokeStyle === value) return;
        if (typeof value !== "string") { throw new TypeError("strokeStyle must be a string"); }
        op_set_stroke_style(this._canvasId, value);
        this._strokeStyle = value;
    }

    get lineWidth() { return this._lineWidth; }
    set lineWidth(value) {
        if (this._lineWidth === value) return;
        if (typeof value !== "number" || value <= 0) { return; }
        op_set_line_width(this._canvasId, value);
        this._lineWidth = value;
    }

    get lineCap() { return this._lineCap; }
    set lineCap(value) {
        if (this._lineCap === value) return;
        const code = LINE_CAP_MAP[value];
        if (code === undefined) return;
        op_set_line_cap(this._canvasId, code);
        this._lineCap = value;
    }

    get lineJoin() { return this._lineJoin; }
    set lineJoin(value) {
        if (this._lineJoin === value) return;
        const code = LINE_JOIN_MAP[value];
        if (code === undefined) return;
        op_set_line_join(this._canvasId, code);
        this._lineJoin = value;
    }

    get miterLimit() { return this._miterLimit; }
    set miterLimit(value) {
        if (this._miterLimit === value) return;
        if (typeof value !== "number" || value <= 0) return;
        op_set_miter_limit(this._canvasId, value);
        this._miterLimit = value;
    }

    get globalAlpha() { return this._globalAlpha; }
    set globalAlpha(value) {
        if (this._globalAlpha === value) return;
        if (typeof value !== "number" || value < 0 || value > 1) return;
        op_set_global_alpha(this._canvasId, value);
        this._globalAlpha = value;
    }

    get font() { return this._font; }
    set font(value) {
        if (this._font === value) return;
        if (typeof value !== "string") { throw new TypeError("font must be a string"); }
        op_set_font(this._canvasId, value);
        this._font = value;
    }

    get textAlign() { return this._textAlign; }
    set textAlign(value) {
        if (this._textAlign === value) return;
        const code = TEXT_ALIGN_MAP[value];
        if (code === undefined) return;
        op_set_text_align(this._canvasId, code);
        this._textAlign = value;
    }

    get textBaseline() { return this._textBaseline; }
    set textBaseline(value) {
        if (this._textBaseline === value) return;
        const code = TEXT_BASELINE_MAP[value];
        if (code === undefined) return;
        op_set_text_baseline(this._canvasId, code);
        this._textBaseline = value;
    }

    // ========== Path methods ==========
    beginPath() { op_begin_path(this._canvasId); }
    closePath() { op_close_path(this._canvasId); }
    moveTo(x, y) { op_move_to(this._canvasId, x, y); }
    lineTo(x, y) { op_line_to(this._canvasId, x, y); }
    quadraticCurveTo(cpx, cpy, x, y) { op_quadratic_curve_to(this._canvasId, cpx, cpy, x, y); }
    bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x, y) { op_bezier_curve_to(this._canvasId, cp1x, cp1y, cp2x, cp2y, x, y); }
    arc(x, y, radius, startAngle, endAngle, counterclockwise = false) { op_arc(this._canvasId, x, y, radius, startAngle, endAngle, counterclockwise); }
    arcTo(x1, y1, x2, y2, radius) { op_arc_to(this._canvasId, x1, y1, x2, y2, radius); }
    rect(x, y, w, h) { op_rect(this._canvasId, x, y, w, h); }
    ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle, counterclockwise = false) { op_ellipse(this._canvasId, x, y, radiusX, radiusY, rotation, startAngle, endAngle, counterclockwise); }

    // ========== Drawing methods ==========
    fill(pathOrFillRule) {
        if (pathOrFillRule !== undefined) { console.warn("fill() with argument is not fully supported"); }
        op_fill(this._canvasId);
    }
    stroke(path) {
        if (path !== undefined) { console.warn("stroke() with argument is not fully supported"); }
        op_stroke(this._canvasId);
    }
    clip(pathOrFillRule) {
        if (pathOrFillRule !== undefined) { console.warn("clip() with argument is not fully supported"); }
        op_clip(this._canvasId);
    }

    // ========== Rectangle methods ==========
    fillRect(x, y, width, height) { op_fill_rect(this._canvasId, x, y, width, height); }
    strokeRect(x, y, width, height) { op_stroke_rect(this._canvasId, x, y, width, height); }
    clearRect(x, y, width, height) { op_clear_rect(this._canvasId, x, y, width, height); }

    // ========== Text methods ==========
    fillText(text, x, y, maxWidth) {
        if (typeof text !== "string") text = String(text);
        op_fill_text(this._canvasId, text, x, y, maxWidth ?? -1);
    }
    strokeText(text, x, y, maxWidth) {
        if (typeof text !== "string") text = String(text);
        op_stroke_text(this._canvasId, text, x, y, maxWidth ?? -1);
    }
    measureText(text) {
        if (typeof text !== "string") text = String(text);
        return op_measure_text(this._canvasId, text);
    }

    // ========== State methods ==========
    save() { op_save(this._canvasId); }
    restore() { op_restore(this._canvasId); }

    // ========== Transform methods ==========
    translate(x, y) { op_translate(this._canvasId, x, y); }
    rotate(angle) { op_rotate(this._canvasId, angle); }
    scale(x, y) { op_scale(this._canvasId, x, y); }
    setTransform(a, b, c, d, e, f) { op_set_transform(this._canvasId, a, b, c, d, e, f); }
    resetTransform() { op_reset_transform(this._canvasId); }
    transform(a, b, c, d, e, f) {
        // transform() multiplies the current matrix; we approximate by setTransform for now
        console.warn("transform() is approximated; use setTransform() for precise control");
        op_set_transform(this._canvasId, a, b, c, d, e, f);
    }
    getTransform() {
        console.warn("getTransform() not implemented, returning identity");
        return { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
    }

    // ========== Image methods ==========
    drawImage(image, ...args) {
        if (!image || !image.loaded) {
            console.warn("image not loaded " + (image?._rid ?? "null") + " " + (image?.src ?? ""));
            return;
        }
        const imgId = image.rid;
        if (args.length === 2) {
            const [dx, dy] = args;
            op_draw_image(this._canvasId, imgId, -1, -1, -1, -1, dx, dy, -1, -1);
        } else if (args.length === 4) {
            const [dx, dy, dw, dh] = args;
            op_draw_image(this._canvasId, imgId, -1, -1, -1, -1, dx, dy, dw, dh);
        } else if (args.length === 8) {
            const [sx, sy, sw, sh, dx, dy, dw, dh] = args;
            op_draw_image(this._canvasId, imgId, sx, sy, sw, sh, dx, dy, dw, dh);
        } else {
            throw new TypeError("drawImage: invalid number of arguments");
        }
    }

    /**
     * Batch draw multiple images for better performance
     * Reduces IPC overhead by sending all draws in a single call
     *
     * @param {Array<{image: Image, sx?: number, sy?: number, sw?: number, sh?: number, dx: number, dy: number, dw?: number, dh?: number}>} draws
     */
    drawImageBatch(draws) {
        if (!Array.isArray(draws) || draws.length === 0) return;

        // Filter out unloaded images and prepare data
        const validDraws = draws.filter(d => d.image && d.image.loaded);
        if (validDraws.length === 0) return;

        // Pack data into Float32Array: 9 floats per draw
        // [image_id, sx, sy, sw, sh, dx, dy, dw, dh]
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

    putImageData(imageData, dx, dy, dirtyX, dirtyY, dirtyWidth, dirtyHeight) {
        console.warn("putImageData() is not implemented");
    }

    // ========== Compositing (stubs) ==========
    get globalCompositeOperation() { return "source-over"; }
    set globalCompositeOperation(value) { console.warn("globalCompositeOperation is not implemented"); }

    // ========== Shadows (stubs) ==========
    get shadowBlur() { return 0; }
    set shadowBlur(value) { console.warn("shadowBlur is not implemented"); }
    get shadowColor() { return "rgba(0,0,0,0)"; }
    set shadowColor(value) { console.warn("shadowColor is not implemented"); }
    get shadowOffsetX() { return 0; }
    set shadowOffsetX(value) { console.warn("shadowOffsetX is not implemented"); }
    get shadowOffsetY() { return 0; }
    set shadowOffsetY(value) { console.warn("shadowOffsetY is not implemented"); }

    // ========== Other stubs ==========
    isPointInPath(x, y, fillRule) { console.warn("isPointInPath not implemented"); return false; }
    isPointInStroke(x, y) { console.warn("isPointInStroke not implemented"); return false; }
    createLinearGradient(x0, y0, x1, y1) { console.warn("createLinearGradient not implemented"); return {}; }
    createRadialGradient(x0, y0, r0, x1, y1, r1) { console.warn("createRadialGradient not implemented"); return {}; }
    createPattern(image, repetition) { console.warn("createPattern not implemented"); return {}; }
    setLineDash(segments) { console.warn("setLineDash not implemented"); }
    getLineDash() { return []; }
    get lineDashOffset() { return 0; }
    set lineDashOffset(value) { console.warn("lineDashOffset not implemented"); }
}

export { CanvasRenderingContext2D };
