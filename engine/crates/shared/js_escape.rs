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
}
