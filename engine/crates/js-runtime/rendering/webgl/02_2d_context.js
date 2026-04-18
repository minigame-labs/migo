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
    op_frame_end_unified,
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
    op_set_text_direction,
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
    // Compositing + gradient + dash
    op_set_composite_operation,
    op_set_line_dash,
    op_set_line_dash_offset,
    op_set_fill_style_gradient,
    op_set_stroke_style_gradient,
    op_set_fill_style_pattern,
    op_set_stroke_style_pattern,
    op_set_shadow_blur,
    op_set_shadow_color,
    op_set_shadow_offset_x,
    op_set_shadow_offset_y,
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
// Text direction constants - match protocol::TextDirection order.
// Canvas 2D spec accepts "inherit" | "ltr" | "rtl"; unknown values
// are treated as "inherit" (browser-compatible no-op).
const TEXT_DIRECTION_MAP = { 'inherit': 0, 'ltr': 1, 'rtl': 2 };

// Composite operation names indexed to stable u8 opcodes consumed by the
// Rust render thread.  The first 11 entries preserve the legacy numbering
// (so pre-existing bytecode keeps the same behaviour); entries 11..25 are
// the advanced / non-separable modes added with the Skia migration.
// See engine/crates/graphics/backend/gl/blend_mode.rs for the canonical
// table.
const _COMPOSITE_OPS = [
    'source-over', 'source-in', 'source-out', 'source-atop',
    'destination-over', 'destination-in', 'destination-out', 'destination-atop',
    'lighter', 'copy', 'xor',
    'multiply', 'screen', 'overlay', 'darken', 'lighten',
    'color-dodge', 'color-burn', 'hard-light', 'soft-light',
    'difference', 'exclusion',
    'hue', 'saturation', 'color', 'luminosity',
];

// Gradient object returned by createLinearGradient / createRadialGradient.
// Collects color stops and sends them to the render thread when assigned
// to fillStyle.
class CanvasGradient {
    constructor(type, canvasId, x0, y0, r0, x1, y1, r1) {
        this._type = type;
        this._canvasId = canvasId;
        this._x0 = x0;
        this._y0 = y0;
        this._r0 = r0;
        this._x1 = x1;
        this._y1 = y1;
        this._r1 = r1;
        this._stops = [];
    }
    addColorStop(offset, color) {
        var off = Number(offset);
        if (!Number.isFinite(off) || off < 0 || off > 1) {
            throw new RangeError("Failed to execute 'addColorStop': offset must be between 0 and 1");
        }
        if (typeof color !== 'string') {
            throw new TypeError("Failed to execute 'addColorStop': color must be a string");
        }
        var parsed = _parseColorToRGBA(color);
        this._stops.push({ offset: off, r: parsed[0], g: parsed[1], b: parsed[2], a: parsed[3] });
        this._stops.sort(function (a, b) { return a.offset - b.offset; });
    }
    // Called internally when this gradient is assigned to fillStyle.
    _apply() {
        if (this._stops.length < 2) return;
        op_set_fill_style_gradient(
            this._canvasId,
            this._type === 'radial' ? 1 : this._type === 'conic' ? 2 : 0,
            this._x0, this._y0, this._r0,
            this._x1, this._y1, this._r1,
            JSON.stringify(this._stops)
        );
    }
    // Called internally when this gradient is assigned to strokeStyle.
    _applyStroke() {
        if (this._stops.length < 2) return;
        op_set_stroke_style_gradient(
            this._canvasId,
            this._type === 'radial' ? 1 : this._type === 'conic' ? 2 : 0,
            this._x0, this._y0, this._r0,
            this._x1, this._y1, this._r1,
            JSON.stringify(this._stops)
        );
    }
}

class CanvasPattern {
    constructor(canvasId, imageRid, repetition) {
        this._canvasId = canvasId;
        this._imageRid = imageRid;
        var rep = repetition == null ? 'repeat' : String(repetition);
        if (rep !== 'repeat' && rep !== 'repeat-x' && rep !== 'repeat-y' && rep !== 'no-repeat') {
            throw new TypeError("Failed to execute 'createPattern': invalid repetition value");
        }
        this._repeatX = rep === 'repeat' || rep === 'repeat-x';
        this._repeatY = rep === 'repeat' || rep === 'repeat-y';
    }
    _applyFill() {
        op_set_fill_style_pattern(this._canvasId, this._imageRid, this._repeatX, this._repeatY);
    }
    _applyStroke() {
        op_set_stroke_style_pattern(this._canvasId, this._imageRid, this._repeatX, this._repeatY);
    }
}

// Full CSS named color table, synced with Rust NAMED_COLORS. Values are [r,g,b,a].
const _NAMED_COLORS = {
    'transparent': [0,0,0,0],
    'aliceblue': [240,248,255,255], 'antiquewhite': [250,235,215,255],
    'aqua': [0,255,255,255], 'aquamarine': [127,255,212,255],
    'azure': [240,255,255,255], 'beige': [245,245,220,255],
    'bisque': [255,228,196,255], 'black': [0,0,0,255],
    'blanchedalmond': [255,235,205,255], 'blue': [0,0,255,255],
    'blueviolet': [138,43,226,255], 'brown': [165,42,42,255],
    'burlywood': [222,184,135,255], 'cadetblue': [95,158,160,255],
    'chartreuse': [127,255,0,255], 'chocolate': [210,105,30,255],
    'coral': [255,127,80,255], 'cornflowerblue': [100,149,237,255],
    'cornsilk': [255,248,220,255], 'crimson': [220,20,60,255],
    'cyan': [0,255,255,255], 'darkblue': [0,0,139,255],
    'darkcyan': [0,139,139,255], 'darkgoldenrod': [184,134,11,255],
    'darkgray': [169,169,169,255], 'darkgreen': [0,100,0,255],
    'darkgrey': [169,169,169,255], 'darkkhaki': [189,183,107,255],
    'darkmagenta': [139,0,139,255], 'darkolivegreen': [85,107,47,255],
    'darkorange': [255,140,0,255], 'darkorchid': [153,50,204,255],
    'darkred': [139,0,0,255], 'darksalmon': [233,150,122,255],
    'darkseagreen': [143,188,143,255], 'darkslateblue': [72,61,139,255],
    'darkslategray': [47,79,79,255], 'darkslategrey': [47,79,79,255],
    'darkturquoise': [0,206,209,255], 'darkviolet': [148,0,211,255],
    'deeppink': [255,20,147,255], 'deepskyblue': [0,191,255,255],
    'dimgray': [105,105,105,255], 'dimgrey': [105,105,105,255],
    'dodgerblue': [30,144,255,255], 'firebrick': [178,34,34,255],
    'floralwhite': [255,250,240,255], 'forestgreen': [34,139,34,255],
    'fuchsia': [255,0,255,255], 'gainsboro': [220,220,220,255],
    'ghostwhite': [248,248,255,255], 'gold': [255,215,0,255],
    'goldenrod': [218,165,32,255], 'gray': [128,128,128,255],
    'green': [0,128,0,255], 'greenyellow': [173,255,47,255],
    'grey': [128,128,128,255], 'honeydew': [240,255,240,255],
    'hotpink': [255,105,180,255], 'indianred': [205,92,92,255],
    'indigo': [75,0,130,255], 'ivory': [255,255,240,255],
    'khaki': [240,230,140,255], 'lavender': [230,230,250,255],
    'lavenderblush': [255,240,245,255], 'lawngreen': [124,252,0,255],
    'lemonchiffon': [255,250,205,255], 'lightblue': [173,216,230,255],
    'lightcoral': [240,128,128,255], 'lightcyan': [224,255,255,255],
    'lightgoldenrodyellow': [250,250,210,255], 'lightgray': [211,211,211,255],
    'lightgreen': [144,238,144,255], 'lightgrey': [211,211,211,255],
    'lightpink': [255,182,193,255], 'lightsalmon': [255,160,122,255],
    'lightseagreen': [32,178,170,255], 'lightskyblue': [135,206,250,255],
    'lightslategray': [119,136,153,255], 'lightslategrey': [119,136,153,255],
    'lightsteelblue': [176,196,222,255], 'lightyellow': [255,255,224,255],
    'lime': [0,255,0,255], 'limegreen': [50,205,50,255],
    'linen': [250,240,230,255], 'magenta': [255,0,255,255],
    'maroon': [128,0,0,255], 'mediumaquamarine': [102,205,170,255],
    'mediumblue': [0,0,205,255], 'mediumorchid': [186,85,211,255],
    'mediumpurple': [147,112,219,255], 'mediumseagreen': [60,179,113,255],
    'mediumslateblue': [123,104,238,255], 'mediumspringgreen': [0,250,154,255],
    'mediumturquoise': [72,209,204,255], 'mediumvioletred': [199,21,133,255],
    'midnightblue': [25,25,112,255], 'mintcream': [245,255,250,255],
    'mistyrose': [255,228,225,255], 'moccasin': [255,228,181,255],
    'navajowhite': [255,222,173,255], 'navy': [0,0,128,255],
    'oldlace': [253,245,230,255], 'olive': [128,128,0,255],
    'olivedrab': [107,142,35,255], 'orange': [255,165,0,255],
    'orangered': [255,69,0,255], 'orchid': [218,112,214,255],
    'palegoldenrod': [238,232,170,255], 'palegreen': [152,251,152,255],
    'paleturquoise': [175,238,238,255], 'palevioletred': [219,112,147,255],
    'papayawhip': [255,239,213,255], 'peachpuff': [255,218,185,255],
    'peru': [205,133,63,255], 'pink': [255,192,203,255],
    'plum': [221,160,221,255], 'powderblue': [176,224,230,255],
    'purple': [128,0,128,255], 'rebeccapurple': [102,51,153,255],
    'red': [255,0,0,255], 'rosybrown': [188,143,143,255],
    'royalblue': [65,105,225,255], 'saddlebrown': [139,69,19,255],
    'salmon': [250,128,114,255], 'sandybrown': [244,164,96,255],
    'seagreen': [46,139,87,255], 'seashell': [255,245,238,255],
    'sienna': [160,82,45,255], 'silver': [192,192,192,255],
    'skyblue': [135,206,235,255], 'slateblue': [106,90,205,255],
    'slategray': [112,128,144,255], 'slategrey': [112,128,144,255],
    'snow': [255,250,250,255], 'springgreen': [0,255,127,255],
    'steelblue': [70,130,180,255], 'tan': [210,180,140,255],
    'teal': [0,128,128,255], 'thistle': [216,191,216,255],
    'tomato': [255,99,71,255], 'turquoise': [64,224,208,255],
    'violet': [238,130,238,255], 'wheat': [245,222,179,255],
    'white': [255,255,255,255], 'whitesmoke': [245,245,245,255],
    'yellow': [255,255,0,255], 'yellowgreen': [154,205,50,255],
};

// Minimal color string to [r,g,b,a] parser.
function _parseColorToRGBA(color) {
    if (typeof color !== 'string') return [0, 0, 0, 255];
    color = color.trim();
    // rgba(r,g,b,a) or rgb(r,g,b)
    var m = color.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+))?\s*\)$/);
    if (m) {
        var a = m[4] !== undefined ? Math.round(parseFloat(m[4]) * 255) : 255;
        return [parseInt(m[1]), parseInt(m[2]), parseInt(m[3]), a];
    }
    // #RRGGBB, #RGB, #RRGGBBAA, #RGBA
    if (color[0] === '#') {
        var hex = color.slice(1);
        if (hex.length === 3) hex = hex[0]+hex[0]+hex[1]+hex[1]+hex[2]+hex[2];
        if (hex.length === 4) hex = hex[0]+hex[0]+hex[1]+hex[1]+hex[2]+hex[2]+hex[3]+hex[3];
        var n = parseInt(hex.substring(0, 6), 16);
        var alpha = hex.length === 8 ? parseInt(hex.substring(6, 8), 16) : 255;
        return [(n >> 16) & 255, (n >> 8) & 255, n & 255, alpha];
    }
    // Named colors
    var named = _NAMED_COLORS[color.toLowerCase()];
    if (named) return named.slice();
    return [0, 0, 0, 255];
}

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

        // Current transform matrix [a, b, c, d, e, f] for getTransform/transform
        this._tm = [1, 0, 0, 1, 0, 0];

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

    // ==================== State setter dedup ====================
    //
    // Every setter below short-circuits when the incoming value is
    // strict-equal to the shadow copy: string (colour / composite),
    // number, or gradient / pattern object identity.  Animation
    // loops that assign the same `ctx.fillStyle` or same
    // `globalAlpha` on every frame stop pushing redundant
    // `SetFillStyle` / `SetGlobalAlpha` commands into the IPC
    // queue -- eliminates 30-70% of Canvas2D command volume on
    // UI-heavy scenes where setter assignment is not hoisted.

    get fillStyle() { return this._fillStyle; }
    set fillStyle(value) {
        if (this._fillStyle === value) return;
        this._fillStyle = value;
        this._frameBegin();
        if (value instanceof CanvasGradient) {
            value._apply();
        } else if (value instanceof CanvasPattern) {
            value._applyFill();
        } else {
            op_set_fill_style(this._canvasId, String(value));
        }
    }

    get strokeStyle() { return this._strokeStyle; }
    set strokeStyle(value) {
        if (this._strokeStyle === value) return;
        this._strokeStyle = value;
        this._frameBegin();
        if (value instanceof CanvasGradient) {
            value._applyStroke();
        } else if (value instanceof CanvasPattern) {
            value._applyStroke();
        } else {
            op_set_stroke_style(this._canvasId, String(value));
        }
    }

    get lineWidth() { return this._lineWidth; }
    set lineWidth(value) {
        if (this._lineWidth === value) return;
        this._lineWidth = value;
        this._frameBegin();
        op_set_line_width(this._canvasId, value);
    }

    get lineCap() { return this._lineCap; }
    set lineCap(value) {
        if (this._lineCap === value) return;
        this._lineCap = value;
        this._frameBegin();
        op_set_line_cap(this._canvasId, LINE_CAP_MAP[value] ?? 0);
    }

    get lineJoin() { return this._lineJoin; }
    set lineJoin(value) {
        if (this._lineJoin === value) return;
        this._lineJoin = value;
        this._frameBegin();
        op_set_line_join(this._canvasId, LINE_JOIN_MAP[value] ?? 0);
    }

    get miterLimit() { return this._miterLimit; }
    set miterLimit(value) {
        if (this._miterLimit === value) return;
        this._miterLimit = value;
        this._frameBegin();
        op_set_miter_limit(this._canvasId, value);
    }

    get globalAlpha() { return this._globalAlpha; }
    set globalAlpha(value) {
        const clamped = Math.max(0, Math.min(1, value));
        if (this._globalAlpha === clamped) return;
        this._globalAlpha = clamped;
        this._frameBegin();
        op_set_global_alpha(this._canvasId, this._globalAlpha);
    }

    get font() { return this._font; }
    set font(value) {
        if (this._font === value) return;
        this._font = value;
        this._frameBegin();
        op_set_font(this._canvasId, value);
    }

    get textAlign() { return this._textAlign; }
    set textAlign(value) {
        if (this._textAlign === value) return;
        this._textAlign = value;
        this._frameBegin();
        op_set_text_align(this._canvasId, TEXT_ALIGN_MAP[value] ?? 0);
    }

    get textBaseline() { return this._textBaseline; }
    set textBaseline(value) {
        if (this._textBaseline === value) return;
        this._textBaseline = value;
        this._frameBegin();
        op_set_text_baseline(this._canvasId, TEXT_BASELINE_MAP[value] ?? 3);
    }

    get direction() { return this._direction || 'inherit'; }
    set direction(value) {
        if (this._direction === value) return;
        this._direction = value;
        this._frameBegin();
        op_set_text_direction(this._canvasId, TEXT_DIRECTION_MAP[value] ?? 0);
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
            tm: this._tm.slice(),
            compositeOp: this._compositeOp,
            lineDash: this._lineDash ? this._lineDash.slice() : null,
            lineDashOffset: this._lineDashOffset,
            shadowBlur: this._shadowBlur,
            shadowColor: this._shadowColor,
            shadowOffsetX: this._shadowOffsetX,
            shadowOffsetY: this._shadowOffsetY,
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
                _tm: state.tm,
                _compositeOp: state.compositeOp,
                _lineDash: state.lineDash,
                _lineDashOffset: state.lineDashOffset,
                _shadowBlur: state.shadowBlur,
                _shadowColor: state.shadowColor,
                _shadowOffsetX: state.shadowOffsetX,
                _shadowOffsetY: state.shadowOffsetY,
            });
        }
        this._frameBegin();
        op_restore(this._canvasId);
    }

    // ==================== Transform Methods ====================

    translate(x, y) {
        this._frameBegin();
        const m = this._tm;
        m[4] += m[0] * x + m[2] * y;
        m[5] += m[1] * x + m[3] * y;
        op_translate(this._canvasId, x, y);
    }

    rotate(angle) {
        this._frameBegin();
        const cos = Math.cos(angle), sin = Math.sin(angle);
        const m = this._tm;
        const a = m[0], b = m[1], c = m[2], d = m[3];
        m[0] = a * cos + c * sin;
        m[1] = b * cos + d * sin;
        m[2] = a * -sin + c * cos;
        m[3] = b * -sin + d * cos;
        op_rotate(this._canvasId, angle);
    }

    scale(x, y) {
        this._frameBegin();
        this._tm[0] *= x; this._tm[1] *= x;
        this._tm[2] *= y; this._tm[3] *= y;
        op_scale(this._canvasId, x, y);
    }

    setTransform(a, b, c, d, e, f) {
        this._frameBegin();
        this._tm[0] = a; this._tm[1] = b;
        this._tm[2] = c; this._tm[3] = d;
        this._tm[4] = e; this._tm[5] = f;
        op_set_transform(this._canvasId, a, b, c, d, e, f);
    }

    resetTransform() {
        this._frameBegin();
        this._tm[0] = 1; this._tm[1] = 0;
        this._tm[2] = 0; this._tm[3] = 1;
        this._tm[4] = 0; this._tm[5] = 0;
        op_reset_transform(this._canvasId);
    }

    transform(a, b, c, d, e, f) {
        // Multiply current matrix: CTM = CTM * [a b c d e f]
        this._frameBegin();
        const m = this._tm;
        const a0 = m[0], b0 = m[1], c0 = m[2], d0 = m[3], e0 = m[4], f0 = m[5];
        m[0] = a0 * a + c0 * b;
        m[1] = b0 * a + d0 * b;
        m[2] = a0 * c + c0 * d;
        m[3] = b0 * c + d0 * d;
        m[4] = a0 * e + c0 * f + e0;
        m[5] = b0 * e + d0 * f + f0;
        op_set_transform(this._canvasId, m[0], m[1], m[2], m[3], m[4], m[5]);
    }

    getTransform() {
        const m = this._tm;
        return { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5] };
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

    // ==================== Compositing ====================
    get globalCompositeOperation() { return this._compositeOp || 'source-over'; }
    set globalCompositeOperation(value) {
        if (this._compositeOp === value) return;
        var idx = _COMPOSITE_OPS.indexOf(value);
        if (idx !== -1) {
            this._compositeOp = value;
            this._frameBegin();
            op_set_composite_operation(this._canvasId, idx);
        }
    }

    // ==================== Shadows ====================
    get shadowBlur() { return this._shadowBlur || 0; }
    set shadowBlur(value) {
        const v = +value || 0;
        if (this._shadowBlur === v) return;
        this._shadowBlur = v;
        this._frameBegin();
        op_set_shadow_blur(this._canvasId, this._shadowBlur);
    }
    get shadowColor() { return this._shadowColor || 'rgba(0,0,0,0)'; }
    set shadowColor(value) {
        if (this._shadowColor === value) return;
        this._shadowColor = value;
        this._frameBegin();
        op_set_shadow_color(this._canvasId, String(value));
    }
    get shadowOffsetX() { return this._shadowOffsetX || 0; }
    set shadowOffsetX(value) {
        const v = +value || 0;
        if (this._shadowOffsetX === v) return;
        this._shadowOffsetX = v;
        this._frameBegin();
        op_set_shadow_offset_x(this._canvasId, this._shadowOffsetX);
    }
    get shadowOffsetY() { return this._shadowOffsetY || 0; }
    set shadowOffsetY(value) {
        const v = +value || 0;
        if (this._shadowOffsetY === v) return;
        this._shadowOffsetY = v;
        this._frameBegin();
        op_set_shadow_offset_y(this._canvasId, this._shadowOffsetY);
    }

    // ==================== Gradient ====================
    createLinearGradient(x0, y0, x1, y1) {
        return new CanvasGradient('linear', this._canvasId, x0, y0, 0, x1, y1, 0);
    }
    createRadialGradient(x0, y0, r0, x1, y1, r1) {
        return new CanvasGradient('radial', this._canvasId, x0, y0, r0, x1, y1, r1);
    }
    createConicGradient(startAngle, cx, cy) {
        return new CanvasGradient('conic', this._canvasId, cx, cy, 0, startAngle, 0, 0);
    }

    // ==================== Line Dash ====================
    // Handled by the Rust render thread via Skia's `SkPathEffect::dash`
    // (see engine/crates/graphics/backend/gl/paint.rs).  Odd-length dash
    // arrays are doubled on the render side, matching the Canvas 2D spec.
    setLineDash(segments) {
        if (!Array.isArray(segments)) return;
        this._lineDash = segments.slice();
        this._frameBegin();
        var buf = new Float32Array(segments);
        op_set_line_dash(this._canvasId, new Uint8Array(buf.buffer));
    }
    getLineDash() { return this._lineDash ? this._lineDash.slice() : []; }
    get lineDashOffset() { return this._lineDashOffset || 0; }
    set lineDashOffset(value) {
        this._lineDashOffset = +value || 0;
        this._frameBegin();
        op_set_line_dash_offset(this._canvasId, this._lineDashOffset);
    }

    // ==================== Other stubs ====================
    isPointInPath() { return false; }
    isPointInStroke() { return false; }
    createPattern(image, repetition) {
        if (!image || !image.loaded) return null;
        return new CanvasPattern(this._canvasId, image.rid, repetition);
    }
}

// Frame-end callback registry. The unified frame-end op builds a single
// interleaved FramePacket from both Canvas2D and GL segments, with
// Materialize barriers at 2D->GL transitions.
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
    op_frame_end_unified();
});

export { CanvasRenderingContext2D, CanvasGradient };
