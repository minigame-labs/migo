const amdModules = Object.create(null);

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
        return () => {
          throw new Error("require() not supported");
        };
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

export { define };
