import { op_download_subpackage } from "ext:core/ops";
import { require as amdRequire } from "ext:host_v8_base/01_amdshim.js";

const noop = () => {};

// ---- game.json config cache ----

const _subpackageByName = new Map();
const _subpackageByRoot = new Map();
let _configLoaded = false;
let _workersRoot = null;

function _normalizeRoot(root) {
    let value = String(root || "").trim();
    value = value.replace(/^\.?\//, "");
    value = value.replace(/^\/+/, "");
    value = value.replace(/\/+$/, "");
    // Reject path traversal segments
    const parts = value.split("/");
    for (let i = 0; i < parts.length; i++) {
        if (parts[i] === "..") throw new Error("invalid path: contains ..");
    }
    return value;
}

function _deriveNameFromRoot(root) {
    const normalized = _normalizeRoot(root);
    if (!normalized) return "";
    const parts = normalized.split("/").filter(Boolean);
    return parts.length > 0 ? parts[parts.length - 1] : "";
}

function _errorText(err) {
    if (typeof err === "string") return err;
    if (err && typeof err.errMsg === "string") return err.errMsg;
    if (err && typeof err.message === "string") return err.message;
    try { return String(err); } catch (_) { return "unknown error"; }
}

function _isNotFoundError(err) {
    const text = _errorText(err).toLowerCase();
    return text.includes("cannot find module")
        || text.includes("cannot read")
        || text.includes("no such file");
}

function _loadGameConfig() {
    if (_configLoaded) return;
    _configLoaded = true;

    let cfg;
    try { cfg = amdRequire("game.json"); } catch (_) { return; }

    const subPackages = Array.isArray(cfg?.subPackages)
        ? cfg.subPackages
        : (Array.isArray(cfg?.subpackages) ? cfg.subpackages : []);

    for (const item of subPackages) {
        if (!item || typeof item !== "object") continue;

        const root = _normalizeRoot(item.root || item.name || "");
        if (!root) continue;

        const name = typeof item.name === "string" && item.name.trim().length > 0
            ? item.name.trim()
            : _deriveNameFromRoot(root);
        if (!name) continue;

        const record = { name, root };
        _subpackageByName.set(name, record);
        _subpackageByRoot.set(root, record);
    }

    // Parse workers config
    const workers = cfg && cfg.workers;
    if (typeof workers === "string") {
        const r = _normalizeRoot(workers);
        if (r) _workersRoot = r;
    } else if (workers && typeof workers === "object" && typeof workers.path === "string") {
        const r = _normalizeRoot(workers.path);
        if (r) _workersRoot = r;
    }
}

function _resolveSubpackage(options) {
    _loadGameConfig();

    const rawName = typeof options.name === "string"
        ? options.name
        : (typeof options.root === "string" ? options.root : "");

    const requested = String(rawName || "").trim();
    if (!requested) throw new Error("name is required");

    if (requested === "__GAME__") {
        return { name: "__GAME__", root: "", fromConfig: true };
    }

    const byName = _subpackageByName.get(requested);
    if (byName) return { ...byName, fromConfig: true };

    const normalized = _normalizeRoot(requested);
    const byRoot = _subpackageByRoot.get(normalized);
    if (byRoot) return { ...byRoot, fromConfig: true };

    // Fallback: not in config, derive root from name
    const root = normalized.startsWith("subpackages/")
        ? normalized
        : `subpackages/${normalized}`;
    const name = _deriveNameFromRoot(root) || requested;
    return { name, root, fromConfig: false };
}

// ---- entry point execution ----

const _loadedSubpackages = new Set();

function _buildEntrypoints(pkg) {
    const list = [];
    function push(path) {
        if (path && !list.includes(path)) list.push(path);
    }

    if (pkg.root) {
        push(`${pkg.root}/game.js`);
        push(`${pkg.root}/index.js`);
        push(`${pkg.root}/main.js`);
    }

    if (pkg.name) {
        push(`${pkg.name}/game.js`);
        push(`${pkg.name}/index.js`);
        push(`subpackages/${pkg.name}/game.js`);
        push(`subpackages/${pkg.name}/index.js`);
    }

    return list;
}

function _subpackageKey(pkg) {
    return `${pkg.name}::${pkg.root}`;
}

function _executeSubpackage(pkg) {
    if (!_tryLocalExecute(pkg)) {
        throw new Error(`subpackage entry not found: ${pkg.name}`);
    }
}

// ---- pending task tracking ----

let _nextRequestId = 1;
const _pendingTasks = new Map();

class SubpackageTask {
    constructor() {
        this._progressListeners = [];
    }

    onProgressUpdate(listener) {
        if (typeof listener === "function") {
            this._progressListeners.push(listener);
        }
    }

    _triggerProgress(progress, totalBytesWritten, totalBytesExpectedToWrite) {
        const payload = { progress, totalBytesWritten, totalBytesExpectedToWrite };
        for (const listener of this._progressListeners) {
            try { listener(payload); } catch (err) {
                console.error("subpackage progress listener error:", err);
            }
        }
    }
}

function _settle(requestId, error) {
    const pending = _pendingTasks.get(requestId);
    if (!pending) return;
    _pendingTasks.delete(requestId);

    if (error) {
        // loadSubpackage: download failed, but files might already be local
        if (pending.executeAfter) {
            try {
                if (_tryLocalExecute(pending.pkg)) {
                    // Local execution succeeded despite download error
                    pending.task._triggerProgress(100, 1, 1);
                    const res = { errMsg: `${pending.apiName}:ok` };
                    pending.success(res);
                    pending.complete(res);
                    return;
                }
            } catch (_) {
                // Local execution also failed, report original download error
            }
        }
        const res = { errMsg: `${pending.apiName}:fail ${error}` };
        pending.fail(res);
        pending.complete(res);
        return;
    }

    // Download succeeded -- execute entry file if needed
    if (pending.executeAfter) {
        try {
            _executeSubpackage(pending.pkg);
        } catch (e) {
            const res = { errMsg: `${pending.apiName}:fail ${_errorText(e)}` };
            pending.fail(res);
            pending.complete(res);
            return;
        }
    }

    pending.task._triggerProgress(100, 1, 1);
    const res = { errMsg: `${pending.apiName}:ok` };
    pending.success(res);
    pending.complete(res);
}

// Local-only fallback when platform service is unavailable.
// Tries to execute entry file directly (all code already on disk).
function _localFallback(requestId) {
    const pending = _pendingTasks.get(requestId);
    if (!pending) return;

    try {
        pending.task._triggerProgress(0, 0, 1);
        _settle(requestId, null);
    } catch (err) {
        _settle(requestId, _errorText(err));
    }
}

// Try to execute a subpackage from local files.
// Returns true if found and executed (or already loaded).
// Returns false if entry not found on disk.
// Throws if entry exists but has a runtime error.
function _tryLocalExecute(pkg) {
    const key = _subpackageKey(pkg);
    if (_loadedSubpackages.has(key)) return true;
    if (pkg.name === "__GAME__") {
        _loadedSubpackages.add(key);
        return true;
    }

    const candidates = _buildEntrypoints(pkg);
    for (const entry of candidates) {
        try {
            amdRequire(entry);
            _loadedSubpackages.add(key);
            return true;
        } catch (err) {
            if (_isNotFoundError(err)) continue;
            throw err; // entry exists but has a runtime error
        }
    }
    return false; // not found locally
}

function _startDownload(apiName, options, pkg, executeAfter) {
    const task = new SubpackageTask();
    const requestId = _nextRequestId++;

    _pendingTasks.set(requestId, {
        task,
        pkg,
        apiName,
        executeAfter,
        success: typeof options.success === "function" ? options.success : noop,
        fail: typeof options.fail === "function" ? options.fail : noop,
        complete: typeof options.complete === "function" ? options.complete : noop,
    });

    // Fast path: if loading and entry file is already on disk, skip download.
    // preDownloadSubpackage (executeAfter=false) always asks the platform,
    // since we cannot check file existence without executing.
    if (executeAfter) {
        try {
            if (_tryLocalExecute(pkg)) {
                // Already loaded -- report success asynchronously
                queueMicrotask(() => _localFallback(requestId));
                return task;
            }
        } catch (e) {
            // Entry found but runtime error -- report failure asynchronously
            queueMicrotask(() => _settle(requestId, _errorText(e)));
            return task;
        }
    }

    try {
        op_download_subpackage(JSON.stringify({
            requestId,
            name: pkg.name,
            root: pkg.root,
        }));
    } catch (_) {
        if (executeAfter) {
            // loadSubpackage: platform unavailable, fall back to local execution
            queueMicrotask(() => _localFallback(requestId));
        } else {
            // preDownloadSubpackage: platform unavailable, report real failure
            queueMicrotask(() => _settle(requestId, "download service not available"));
        }
    }

    return task;
}

// ---- EvalScript callbacks (called from JNI inbound) ----

function _internalOnSubpackageProgress(resultJson) {
    let data;
    try { data = JSON.parse(resultJson); } catch (_) { return; }
    const pending = _pendingTasks.get(data.requestId);
    if (!pending) return;
    pending.task._triggerProgress(
        data.progress || 0,
        data.totalBytesWritten || 0,
        data.totalBytesExpectedToWrite || 0,
    );
}

function _internalOnSubpackageResult(resultJson) {
    let data;
    try { data = JSON.parse(resultJson); } catch (_) { return; }
    _settle(data.requestId, data.error || null);
}

// ---- public API ----

function loadSubpackage(options = {}) {
    let pkg;
    try {
        pkg = _resolveSubpackage(options);
    } catch (e) {
        // Return a task that immediately fails
        const task = new SubpackageTask();
        const fail = typeof options.fail === "function" ? options.fail : noop;
        const complete = typeof options.complete === "function" ? options.complete : noop;
        queueMicrotask(() => {
            const res = { errMsg: `loadSubpackage:fail ${_errorText(e)}` };
            fail(res);
            complete(res);
        });
        return task;
    }
    return _startDownload("loadSubpackage", options, pkg, true);
}

function preDownloadSubpackage(options = {}) {
    const packageType = typeof options.packageType === "string" && options.packageType.length > 0
        ? options.packageType
        : "normal";

    if (packageType === "workers") {
        _loadGameConfig();
        if (!_workersRoot) {
            const task = new SubpackageTask();
            const fail = typeof options.fail === "function" ? options.fail : noop;
            const complete = typeof options.complete === "function" ? options.complete : noop;
            queueMicrotask(() => {
                const res = { errMsg: "preDownloadSubpackage:fail workers subpackage is not configured" };
                fail(res);
                complete(res);
            });
            return task;
        }
        const pkg = { name: _workersRoot, root: _workersRoot, fromConfig: true };
        return _startDownload("preDownloadSubpackage", options, pkg, false);
    }

    if (packageType !== "normal") {
        const task = new SubpackageTask();
        const fail = typeof options.fail === "function" ? options.fail : noop;
        const complete = typeof options.complete === "function" ? options.complete : noop;
        queueMicrotask(() => {
            const res = { errMsg: `preDownloadSubpackage:fail invalid packageType: ${packageType}` };
            fail(res);
            complete(res);
        });
        return task;
    }

    let pkg;
    try {
        pkg = _resolveSubpackage(options);
    } catch (e) {
        const task = new SubpackageTask();
        const fail = typeof options.fail === "function" ? options.fail : noop;
        const complete = typeof options.complete === "function" ? options.complete : noop;
        queueMicrotask(() => {
            const res = { errMsg: `preDownloadSubpackage:fail ${_errorText(e)}` };
            fail(res);
            complete(res);
        });
        return task;
    }

    // preDownloadSubpackage only allows subpackages defined in game.json
    if (!pkg.fromConfig) {
        const task = new SubpackageTask();
        const fail = typeof options.fail === "function" ? options.fail : noop;
        const complete = typeof options.complete === "function" ? options.complete : noop;
        const reqName = options.name || options.root || "";
        queueMicrotask(() => {
            const res = { errMsg: `preDownloadSubpackage:fail subpackage not configured: ${reqName}` };
            fail(res);
            complete(res);
        });
        return task;
    }

    return _startDownload("preDownloadSubpackage", options, pkg, false);
}

export {
    loadSubpackage,
    preDownloadSubpackage,
    _internalOnSubpackageProgress,
    _internalOnSubpackageResult,
};
