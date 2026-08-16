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

  let moduleObj = { exports: {} };
  let exportsObj = moduleObj.exports;

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

// NOTE: do NOT set define.amd = true here.
// On a common mini-game platform's Android runtime, the platform-provided define() does NOT have .amd set.
// Setting define.amd = true causes UMD modules inside browserify/webpack
// bundles (e.g. tslib) to mistakenly take the AMD path, registering into
// the global AMD registry instead of populating module.exports. This leaves
// the bundle's internal require() with an empty exports object.
//
// The amdshim's own AMD detection (line ~101) uses source-code string
// matching (code.includes("define.amd")), not a runtime check, so this
// removal does not affect amdshim's module loading logic.

function require(name) {
  // 1. Check AMD module registry
  if (amdModules[name] !== undefined) return amdModules[name];

  // 2. Check require cache (by name)
  if (requireCache[name] !== undefined) return requireCache[name];

  // 3. Load from file system
  const referrerDir = _requireDirStack.length > 0 ? _requireDirStack[_requireDirStack.length - 1] : "";
  const { code, abs_path: absPath, dir } = op_require_resolve_and_read(name, referrerDir);

  // Check cache by absolute path (may differ from `name`)
  if (requireCache[absPath] !== undefined) return requireCache[absPath];

  // JSON files are returned as parsed objects (matches Node.js behavior)
  if (absPath.endsWith(".json")) {
    const parsed = JSON.parse(code);
    requireCache[name] = parsed;
    requireCache[absPath] = parsed;
    return parsed;
  }

  // Set up module object; exports starts as the same reference (CJS invariant)
  const moduleObj = { exports: {} };

  // Pre-cache before execution to handle circular dependencies.
  // If module B requires module A while A is still executing,
  // B gets A's partially-filled exports object (Node.js behavior).
  requireCache[name] = moduleObj.exports;
  requireCache[absPath] = moduleObj.exports;

  _requireDirStack.push(dir);
  try {
    // Check if the loaded code is an AMD module
    if (code.includes("define.amd") || code.includes("typeof define")) {
      // Save/restore _lastDefinedModule to handle nested define() calls
      const prevDefined = globalThis._lastDefinedModule;
      const exec = new Function("define", "require", "module", "exports",
        code + "\n//# sourceURL=" + absPath);
      exec(define, require, moduleObj, moduleObj.exports);
      const val = globalThis._lastDefinedModule;
      globalThis._lastDefinedModule = prevDefined;
      requireCache[name] = val;
      requireCache[absPath] = val;
      return val;
    }

    // Regular CJS module - sourceURL enables meaningful stack traces
    const exec = new Function("require", "module", "exports", "__filename", "__dirname",
      code + "\n//# sourceURL=" + absPath);
    exec(require, moduleObj, moduleObj.exports, absPath, dir);
  } catch (e) {
    // Remove from cache on error so the module can be retried
    delete requireCache[name];
    delete requireCache[absPath];
    throw e;
  } finally {
    _requireDirStack.pop();
  }

  // Update cache: module.exports may have been reassigned
  // (e.g. `module.exports = MyClass`)
  const result = moduleObj.exports;
  requireCache[name] = result;
  requireCache[absPath] = result;

  return result;
}

export { define, require };
