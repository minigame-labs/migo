//! CommonJS compatibility detection utilities.
//!
//! Provides heuristic detection of CommonJS module patterns and
//! wrapping of CJS source into AMD `define()` calls. Used by both
//! the main-thread module loader and the worker module loader.

/// Detect whether source code uses CommonJS module patterns.
///
/// Scans line-by-line (skipping single-line comments) for CJS indicators
/// (`require(`, `module.exports`, `exports.`) and ESM indicators at
/// statement level (`import ...`, `export ...`).
///
/// Returns `true` only if CJS patterns are found and no ESM syntax is present.
pub fn is_cjs(code: &str) -> bool {
    let mut has_cjs = false;
    let mut has_esm = false;

    for line in code.lines() {
        let t = line.trim();

        // Skip comment lines
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }

        // CJS patterns
        if !has_cjs
            && (t.contains("require(") || t.contains("module.exports") || t.contains("exports."))
        {
            has_cjs = true;
        }

        // ESM patterns at statement level (beginning of line)
        if !has_esm
            && (t.starts_with("import ")
                || t.starts_with("import\"")
                || t.starts_with("import'")
                || t.starts_with("import{")
                || t.starts_with("export ")
                || t.starts_with("export{")
                || t.starts_with("export default"))
        {
            has_esm = true;
        }

        if has_cjs && has_esm {
            break;
        }
    }

    has_cjs && !has_esm
}

/// Wrap CommonJS source in an AMD `define` call.
///
/// The resulting code uses `define(["require", "exports", "module"], ...)` so
/// the existing AMD shim can handle module registration. An ESM `export default`
/// is appended so the Deno module loader picks up the result.
pub fn wrap_cjs(code: &str) -> String {
    format!(
        "define([\"require\", \"exports\", \"module\"], function(require, exports, module) {{\n{code}\n}});\nexport default globalThis._lastDefinedModule;\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cjs_require() {
        assert!(is_cjs("const foo = require('bar');"));
    }

    #[test]
    fn detects_cjs_module_exports() {
        assert!(is_cjs("module.exports = MyClass;"));
    }

    #[test]
    fn detects_cjs_exports_dot() {
        assert!(is_cjs("exports.foo = 42;"));
    }

    #[test]
    fn rejects_esm_with_import() {
        assert!(!is_cjs("import foo from 'bar';\nconst x = require('y');"));
    }

    #[test]
    fn rejects_esm_with_export() {
        assert!(!is_cjs("const x = require('y');\nexport default x;"));
    }

    #[test]
    fn ignores_require_in_comments() {
        assert!(!is_cjs("// require('foo')"));
    }

    #[test]
    fn ignores_import_in_comments() {
        // CJS code with an import mentioned in a comment should still be detected
        assert!(is_cjs("const x = require('y');\n// import foo from 'bar';"));
    }

    #[test]
    fn plain_code_is_not_cjs() {
        assert!(!is_cjs("console.log('hello');"));
    }

    #[test]
    fn wrap_produces_valid_amd() {
        let wrapped = wrap_cjs("module.exports = 42;");
        assert!(wrapped.contains("define(["));
        assert!(wrapped.contains("module.exports = 42;"));
        assert!(wrapped.contains("export default globalThis._lastDefinedModule;"));
    }
}
