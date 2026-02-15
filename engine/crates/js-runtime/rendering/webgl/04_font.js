const { core } = Deno;
const { ops } = core;

/**
 * loadFont(path)
 *
 * Loads a custom font file and returns the font family name.
 * The returned family name can be used in canvas font strings
 * (e.g., ctx.font = "16px custom-font").
 *
 * @param {string} path - Path to the font file (relative to game code directory).
 * @returns {string} Font family name, or empty string on failure.
 */
const loadFont = (path) => {
    if (typeof path !== 'string' || path.length === 0) {
        console.error('loadFont: path must be a non-empty string');
        return '';
    }
    return ops.op_load_font(path);
};

/**
 * getTextLineHeight(object)
 *
 * Returns the line height of text rendered with the given font configuration.
 *
 * @param {object} object
 * @param {string} [object.fontStyle='normal'] - Font style ('normal' or 'italic').
 * @param {string} [object.fontWeight='normal'] - Font weight ('normal' or 'bold').
 * @param {number} [object.fontSize=16] - Font size in pixels.
 * @param {string} [object.fontFamily='sans-serif'] - Font family name.
 * @param {string} [object.text=''] - Not used for measurement, kept for API compat.
 * @returns {number} Line height in pixels.
 */
const getTextLineHeight = (object) => {
    const obj = object || {};
    const fontStyle = obj.fontStyle || 'normal';
    const fontWeight = obj.fontWeight || 'normal';
    const fontSize = typeof obj.fontSize === 'number' ? obj.fontSize : 16;
    const fontFamily = obj.fontFamily || 'sans-serif';

    const bold = fontWeight === 'bold' || fontWeight === '700';
    const italic = fontStyle === 'italic';

    return ops.op_get_text_line_height(fontFamily, fontSize, bold, italic);
};

export { loadFont, getTextLineHeight };
