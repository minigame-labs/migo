import {
    op_create_context_2d,
    op_arc,
    op_set_fill_style,
    op_set_stroke_style,
    op_set_line_width,
    op_fill_rect,
    op_stroke_rect,
    op_clear_rect,
    op_begin_path,
    op_move_to,
    op_line_to,
    op_fill,
    op_stroke,
    op_fill_text,
    op_stroke_text,
    op_set_font,
    op_draw_image,
} from "ext:core/ops";

class CanvasRenderingContext2D {
    constructor(canvas) {
        this._canvas = canvas;
        this._canvasId = canvas._rid;

        this._ctxId = op_create_context_2d(this._canvasId);
        if (this._ctxId < 0) {
            console.error("Failed to create 2d context");
        }
    }

    get canvas() {
        return this._canvas;
    }

    get fillStyle() {
        return this._fillStyle || "rgb(0,0,0)";
    }

    set fillStyle(value) {
        if (this._fillStyle === value) return;

        if (typeof value !== "string") {
            // TODO: support CanvasGradient and CanvasPattern
            throw new TypeError("fillStyle must be a string");
        }

        op_set_fill_style(this._canvasId, value);
        this._fillStyle = value;
    }

    get strokeStyle() {
        return this._strokeStyle || "rgb(0,0,0)";
    }

    set strokeStyle(value) {
        if (this._strokeStyle === value) return;

        if (typeof value !== "string") {
            // TODO: support CanvasGradient and CanvasPattern
            throw new TypeError("strokeStyle must be a string");
        }

        op_set_stroke_style(this._canvasId, value);
        this._strokeStyle = value;
    }

    get lineWidth() {
        return this._lineWidth || 1;
    }

    set lineWidth(value) {
        if (this._lineWidth === value) return;

        if (typeof value !== "number" || value <= 0) {
            throw new TypeError("lineWidth must be a positive number");
        }

        op_set_line_width(this._canvasId, value);
        this._lineWidth = value;
    }

    get font() {
        return this._font || "10px sans-serif";
    }

    set font(value) {
        if (this._font === value) return;

        if (typeof value !== "string") {
            throw new TypeError("font must be a string");
        }

        op_set_font(this._canvasId, value);
        this._font = value;
    }

    arc(x, y, radius, startAngle, endAngle, counterclockwise = false) {
        op_arc(this._canvasId, x, y, radius, startAngle, endAngle, counterclockwise);
    }

    fillRect(x, y, width, height) {
        op_fill_rect(this._canvasId, x, y, width, height);
    }

    strokeRect(x, y, width, height) {
        op_stroke_rect(this._canvasId, x, y, width, height);
    }

    clearRect(x, y, width, height) {
        op_clear_rect(this._canvasId, x, y, width, height);
    }

    beginPath() {
        op_begin_path(this._canvasId);
    }

    moveTo(x, y) {
        op_move_to(this._canvasId, x, y);
    }

    lineTo(x, y) {
        op_line_to(this._canvasId, x, y);
    }

    // TODO: support fill with Path2D
    fill(value) {
        if (value !== undefined) {
            console.warn("fill() with argument is not supported yet");
            throw new Error("Not implemented");
        }
        op_fill(this._canvasId);
    }

    // TODO: support stroke with Path2D
    stroke(value) {
        if (value !== undefined) {
            console.warn("stroke() with argument is not supported yet");
            throw new Error("Not implemented");
        }
        op_stroke(this._canvasId);
    }

    fillText(text, x, y, maxWidth) {
        if (typeof text !== "string") text = String(text);
        op_fill_text(this._canvasId, text, x, y, maxWidth ?? -1);
    }

    strokeText(text, x, y, maxWidth) {
        if (typeof text !== "string") text = String(text);
        op_stroke_text(this._canvasId, text, x, y, maxWidth ?? -1);
    }

    // drawImage overloads:
    // drawImage(image, dx, dy)
    // drawImage(image, dx, dy, dw, dh)
    // drawImage(image, sx, sy, sw, sh, dx, dy, dw, dh)
    drawImage(image, ...args) {
        if (!image || !image.loaded) {
            console.warn("image not loaded " + (image?._rid ?? "null") + " " + (image?.src ?? ""));
            return;
        }

        const imgId = image.rid;

        if (args.length === 2) {
            const [dx, dy] = args;
            // src defaults: -1 indicates full
            op_draw_image(this._canvasId, imgId, -1, -1, -1, -1, dx, dy, -1, -1);
            return;
        }

        if (args.length === 4) {
            const [dx, dy, dw, dh] = args;
            op_draw_image(this._canvasId, imgId, -1, -1, -1, -1, dx, dy, dw, dh);
            return;
        }

        if (args.length === 8) {
            const [sx, sy, sw, sh, dx, dy, dw, dh] = args;
            op_draw_image(this._canvasId, imgId, sx, sy, sw, sh, dx, dy, dw, dh);
            return;
        }

        throw new TypeError("drawImage: invalid number of arguments");
    }
}

export { CanvasRenderingContext2D };
