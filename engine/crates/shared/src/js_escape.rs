/// Escape a string for safe interpolation into a JSON double-quoted string.
///
/// Handles: backslash, double quote, newlines, carriage return, and tab.
pub fn escape_for_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape a string for safe interpolation into a JS single-quoted string literal.
///
/// Handles all characters that could break out of the string or inject code:
/// backslash, single quote, newlines, null, backtick, dollar sign, and
/// Unicode line separators (U+2028, U+2029).
pub fn escape_for_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '`' => out.push_str("\\`"),
            '$' => out.push_str("\\$"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

/// JS expression that resolves the host-bridge holder object at runtime.
///
/// The runtime relocates all `_internal*` host-bridge hooks off the
/// game-visible global onto a Symbol-keyed holder (see `99_main.js`). Host
/// callbacks delivered through the EvalScript channel must therefore qualify
/// the hook name with this prefix instead of calling a bare global identifier.
///
/// Keep this in sync with the Symbol key used in `99_main.js` and the lookup
/// in `runtime-v8/src/js_bindings.rs`.
pub const HOST_BRIDGE_EXPR: &str = "globalThis[Symbol.for('Migo.hostBridge')]";

/// Build a complete `<holder>.callbackName('escaped_json');` JS source string
/// in a single allocation.
///
/// `callback_name` is the bare hook name (e.g. `_internalOnLoginResult`); this
/// function prefixes it with [`HOST_BRIDGE_EXPR`] so the call resolves the hook
/// from the Symbol-keyed host-bridge holder rather than the global scope (the
/// hooks are no longer exposed as global identifiers -- see `99_main.js`).
///
/// This combines the work of `escape_for_js_string` + `format!` into one pass,
/// eliminating the intermediate escaped String allocation. The escape rules are
/// identical to `escape_for_js_string`.
pub fn build_eval_script(callback_name: &str, json: &str) -> String {
    // HOST_BRIDGE_EXPR + '.' + name + "('" + escaped + "');"
    let mut out = String::with_capacity(
        HOST_BRIDGE_EXPR.len() + 1 + callback_name.len() + json.len() + 5 + 16,
    );
    out.push_str(HOST_BRIDGE_EXPR);
    out.push('.');
    out.push_str(callback_name);
    out.push_str("('");
    for c in json.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '`' => out.push_str("\\`"),
            '$' => out.push_str("\\$"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out.push_str("');");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_unchanged() {
        assert_eq!(escape_for_js_string("hello"), "hello");
    }

    #[test]
    fn single_quote_escaped() {
        assert_eq!(escape_for_js_string("it's"), "it\\'s");
    }

    #[test]
    fn backslash_escaped() {
        assert_eq!(escape_for_js_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn newline_escaped() {
        assert_eq!(escape_for_js_string("a\nb"), "a\\nb");
    }

    #[test]
    fn carriage_return_escaped() {
        assert_eq!(escape_for_js_string("a\rb"), "a\\rb");
    }

    #[test]
    fn backtick_escaped() {
        assert_eq!(escape_for_js_string("a`b"), "a\\`b");
    }

    #[test]
    fn dollar_escaped() {
        assert_eq!(escape_for_js_string("$var"), "\\$var");
    }

    #[test]
    fn null_escaped() {
        assert_eq!(escape_for_js_string("a\0b"), "a\\0b");
    }

    #[test]
    fn u2028_line_separator_escaped() {
        assert_eq!(escape_for_js_string("a\u{2028}b"), "a\\u2028b");
    }

    #[test]
    fn u2029_paragraph_separator_escaped() {
        assert_eq!(escape_for_js_string("a\u{2029}b"), "a\\u2029b");
    }

    #[test]
    fn combined_escapes() {
        assert_eq!(
            escape_for_js_string("it's a `$100` note\n"),
            "it\\'s a \\`\\$100\\` note\\n"
        );
    }

    #[test]
    fn empty_string() {
        assert_eq!(escape_for_js_string(""), "");
    }

    #[test]
    fn all_special_chars() {
        assert_eq!(escape_for_js_string("\\'\n\r"), "\\\\\\'\\n\\r");
    }

    // ---- build_eval_script tests ----

    #[test]
    fn build_eval_script_plain_json() {
        assert_eq!(
            build_eval_script("_internalOnResult", r#"{"ok":true}"#),
            format!("{HOST_BRIDGE_EXPR}._internalOnResult('{{\"ok\":true}}');"),
        );
    }

    #[test]
    fn build_eval_script_json_with_single_quotes() {
        assert_eq!(
            build_eval_script("_internalOnResult", r#"{"msg":"it's done"}"#),
            format!("{HOST_BRIDGE_EXPR}._internalOnResult('{{\"msg\":\"it\\'s done\"}}');"),
        );
    }

    #[test]
    fn build_eval_script_empty_json() {
        assert_eq!(
            build_eval_script("_internalOnResult", ""),
            format!("{HOST_BRIDGE_EXPR}._internalOnResult('');"),
        );
    }

    #[test]
    fn build_eval_script_matches_escape_plus_format() {
        // Verify that build_eval_script produces identical output to the
        // two-step escape_for_js_string + format! approach.
        let json = r#"{"path":"C:\\Users\\test","note":"line1\nline2"}"#;
        let callback = "_internalOnChooseImageResult";
        let expected = format!(
            "{HOST_BRIDGE_EXPR}.{}('{}');",
            callback,
            escape_for_js_string(json)
        );
        assert_eq!(build_eval_script(callback, json), expected);
    }

    #[test]
    fn build_eval_script_all_special_chars() {
        let json = "\\'\n\r\0`$\u{2028}\u{2029}";
        let callback = "_cb";
        let expected = format!(
            "{HOST_BRIDGE_EXPR}.{}('{}');",
            callback,
            escape_for_js_string(json)
        );
        assert_eq!(build_eval_script(callback, json), expected);
    }
}
