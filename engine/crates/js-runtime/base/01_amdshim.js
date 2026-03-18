import { op_require_resolve_and_read } from "ext:core/ops";

const amdModules = Object.create(null);
const requireCache = Object.create(null);

// Current directory stack for nested require() calls
let _requireDirStack = [];

function define(name, deps, factory) {
  if (typeof name === "function") {
    factory = name;
    name = `__anon_${Date.now()}_${Math.random()}`;
    deps = [];
  } else if (Array.isArray(name) && typeof deps === "function") {
    factory = deps;
    deps = name;
    name = `__anon_${Date.now()}_${Math.random()}`;
  } else if (typeof name === "string" && typeof deps === "function") {
    factory = deps;
    deps = [];
  } else if (typeof name !== "string") {
    factory = name;
    deps = [];
    name = `__anon_${Date.now()}_${Math.random()}`;
  }

  let exportsObj = {};
  let moduleObj = { exports: {} };

  const resolved = (deps || []).map((dep) => {
    switch (dep) {
      case "require":
        return require;
      case "exports":
        return exportsObj;
      case "module":
        return moduleObj;
      default:
        return amdModules[dep];
    }
  });

  let result;
  try {
    result = typeof factory === "function" ? factory(...resolved) : factory;
  } catch (err) {
    console.error("[define] factory error in", name, err);
    throw err;
  }

  let final = result;
  if (final == null || (typeof final === "object" && Object.keys(final).length === 0)) {
    if (moduleObj.exports && Object.keys(moduleObj.exports).length > 0) {
      final = moduleObj.exports;
    } else if (Object.keys(exportsObj).length > 0) {
      final = exportsObj;
    }
  }

  amdModules[name] = final;
  globalThis._lastDefinedModule = final;
}

define.amd = true;

function require(name) {
  // 1. Check AMD module registry
  if (amdModules[name] !== undefined) return amdModules[name];

  // 2. Check require cache (by absolute path, filled after first load)
  if (requireCache[name] !== undefined) return requireCache[name];

  // 3. Load from file system
  const referrerDir = _requireDirStack.length > 0 ? _requireDirStack[_requireDirStack.length - 1] : "";
  const { code, abs_path: absPath, dir } = op_require_resolve_and_read(name, referrerDir);

  // Check cache by absolute path (may differ from `name`)
  if (requireCache[absPath] !== undefined) return requireCache[absPath];

  // Execute the module
  const moduleObj = { exports: {} };
  const exportsObj = moduleObj.exports;

  _requireDirStack.push(dir);
  try {
    // Check if it's an AMD module
    if (code.includes("define.amd") || code.includes("typeof define")) {
      const fn = new Function("define", "require", "module", "exports", code);
      fn(define, require, moduleObj, exportsObj);
      const val = globalThis._lastDefinedModule;
      requireCache[name] = val;
      requireCache[absPath] = val;
      return val;
    }

    // Regular CJS module
    const fn = new Function("require", "module", "exports", "__filename", "__dirname", code);
    fn(require, moduleObj, exportsObj, absPath, dir);
  } finally {
    _requireDirStack.pop();
  }

  const result = moduleObj.exports;
  requireCache[name] = result;
  requireCache[absPath] = result;
  return result;
}

export { define, require };
