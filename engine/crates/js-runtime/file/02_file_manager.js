import {
  op_access, op_access_sync,
  op_write_or_append_file, op_write_or_append_file_sync,
  op_open_file, op_open_file_sync,
  op_close_file, op_close_file_sync,
  op_copy_file, op_copy_file_sync,
  op_fstat, op_fstat_sync,
  op_ftruncate, op_ftruncate_sync,
  op_mkdir, op_mkdir_sync,
  op_readdir, op_readdir_sync,
  op_unlink, op_unlink_sync,
  op_rename, op_rename_sync,
  op_rmdir, op_rmdir_sync,
  op_stat, op_stat_sync,
  op_write_file, op_write_file_sync,
  op_read_compressed_file, op_read_compressed_file_sync,
  op_read_fd, op_read_fd_sync,
  op_read_file, op_read_file_sync,
  op_read_zip_entry,
  op_unzip,
} from "ext:core/ops";

import { core, primordials } from "ext:core/mod.js";
import { FileStats, Stats } from "./02_file_stats.js";

const { Error } = primordials;

class IOError extends Error {
  constructor(msg) {
    super(msg);
    this.name = "IOError";
  }
}
core.registerErrorClass("IOError", IOError);

function extractErrText(err) {
  if (err == null) return "unknown error";
  if (typeof err === "string") return err;
  if (typeof err?.errMsg === "string") return err.errMsg;
  if (typeof err?.message === "string") return err.message;
  try {
    return String(err);
  } catch {
    return "unknown error";
  }
}

function okObj(okMsg, payload) {
  // Always return an object with errMsg (miniapp style)
  if (payload == null) return { errMsg: okMsg };
  if (typeof payload === "object") {
    if (payload.errMsg == null) payload.errMsg = okMsg;
    return payload;
  }
  return { errMsg: okMsg, result: payload };
}

function wrapAsync(promise, okMsg, failPrefix, { success, fail, complete }) {
  Promise.resolve(promise)
    .then((payload) => {
      const out = okObj(okMsg, payload);
      success?.(out);
      complete?.(out);
    })
    .catch((err) => {
      const msg = `${failPrefix}:fail ${extractErrText(err)}`;
      const out = { errMsg: msg };
      fail?.(out);
      complete?.(out);
    });
}

function wrapSync(fn, failPrefix) {
  try {
    return fn();
  } catch (err) {
    throw { errMsg: `${failPrefix}:fail ${extractErrText(err)}` };
  }
}

function toUint8Array(data, offset = 0, length) {
  if (typeof data === "string") {
    return { data_str: data, data_buf: null };
  }
  if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) {
    const buf = data instanceof ArrayBuffer
      ? new Uint8Array(data)
      : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);

    const start = offset >>> 0;
    const end = length !== undefined ? Math.min(start + (length >>> 0), buf.length) : buf.length;
    return { data_buf: buf.subarray(start, end), data_str: null };
  }
  throw new IOError("write: invalid data type (expected string or ArrayBuffer/TypedArray)");
}

function ensureFd(fd) {
  const n = Number(fd);
  if (!Number.isFinite(n)) throw new IOError("invalid fd");
  return n;
}

const SAVE_FILE_DIR = "/user";
let _saveFileCounter = 0;

function getPathExtension(path) {
  if (typeof path !== "string" || path.length === 0) {
    return "";
  }
  const slash = path.lastIndexOf("/");
  const dot = path.lastIndexOf(".");
  if (dot <= slash || dot === path.length - 1) {
    return "";
  }
  return path.slice(dot);
}

function makeSavedFilePath(tempFilePath) {
  const ts = Date.now().toString(36);
  const seq = (++_saveFileCounter).toString(36);
  const ext = getPathExtension(tempFilePath);
  return `${SAVE_FILE_DIR}/saved_${ts}_${seq}${ext}`;
}

function resolveSavedFilePath(filePath, tempFilePath) {
  if (typeof filePath === "string" && filePath.length > 0) {
    return filePath;
  }
  return makeSavedFilePath(tempFilePath);
}

class BaseFileManager {
  //
  // access
  //
  static access({ path, success, fail, complete }) {
    wrapAsync(op_access(path).then(() => undefined), "access:ok", "access", { success, fail, complete });
  }

  static accessSync(path_or_obj) {
    const path = typeof path_or_obj === "object" && path_or_obj !== null ? path_or_obj.path : path_or_obj;
    return wrapSync(() => op_access_sync(path), "accessSync");
  }

  //
  // writeFile / appendFile (path)
  //
  static async writeFile({ filePath, data, encoding = "utf8", success, fail, complete }) {
    BaseFileManager.#writeFileCommon({ filePath, data, encoding, append: false, success, fail, complete });
  }

  static async appendFile({ filePath, data, encoding = "utf8", success, fail, complete }) {
    BaseFileManager.#writeFileCommon({ filePath, data, encoding, append: true, success, fail, complete });
  }

  static writeFileSync(filePath_or_obj, data_arg, encoding_arg) {
    let filePath, data, encoding;
    if (typeof filePath_or_obj === "object" && filePath_or_obj !== null && !ArrayBuffer.isView(filePath_or_obj) && !(filePath_or_obj instanceof ArrayBuffer)) {
      ({ filePath, data, encoding = "utf8" } = filePath_or_obj);
    } else {
      filePath = filePath_or_obj;
      data = data_arg;
      encoding = encoding_arg || "utf8";
    }
    const { data_buf, data_str } = toUint8Array(data);
    wrapSync(() => {
      const ok = op_write_or_append_file_sync(filePath, data_buf, data_str, encoding, false);
      if (!ok) throw new IOError("unknown error");
      return undefined;
    }, "writeFileSync");
  }

  static appendFileSync(filePath_or_obj, data_arg, encoding_arg) {
    let filePath, data, encoding;
    if (typeof filePath_or_obj === "object" && filePath_or_obj !== null && !ArrayBuffer.isView(filePath_or_obj) && !(filePath_or_obj instanceof ArrayBuffer)) {
      ({ filePath, data, encoding = "utf8" } = filePath_or_obj);
    } else {
      filePath = filePath_or_obj;
      data = data_arg;
      encoding = encoding_arg || "utf8";
    }
    const { data_buf, data_str } = toUint8Array(data);
    wrapSync(() => {
      const ok = op_write_or_append_file_sync(filePath, data_buf, data_str, encoding, true);
      if (!ok) throw new IOError("unknown error");
      return undefined;
    }, "appendFileSync");
  }

  static #writeFileCommon({ filePath, data, encoding, append, success, fail, complete }) {
    let data_buf, data_str;
    try {
      ({ data_buf, data_str } = toUint8Array(data));
    } catch (err) {
      const msg = `${append ? "appendFile" : "writeFile"}:fail ${extractErrText(err)}`;
      const out = { errMsg: msg };
      fail?.(out);
      complete?.(out);
      return;
    }

    const prefix = append ? "appendFile" : "writeFile";
    const p = op_write_or_append_file(filePath, data_buf, data_str, encoding, append)
      .then((ok) => {
        if (!ok) throw new IOError("unknown error");
        return undefined;
      });

    wrapAsync(p, `${prefix}:ok`, prefix, { success, fail, complete });
  }

  //
  // open / close
  //
  static open({ filePath, flag = "r", success, fail, complete }) {
    wrapAsync(
      op_open_file(filePath, flag).then((fd) => ({ fd: String(fd) })),
      "open:ok",
      "open",
      { success, fail, complete }
    );
  }

  static openSync({ filePath, flag = "r" }) {
    return wrapSync(() => ({ fd: String(op_open_file_sync(filePath, flag)) }), "openSync");
  }

  static close({ fd, success, fail, complete }) {
    wrapAsync(op_close_file(ensureFd(fd)), "close:ok", "close", { success, fail, complete });
  }

  static closeSync({ fd }) {
    wrapSync(() => op_close_file_sync(ensureFd(fd)), "closeSync");
  }

  //
  // copy / rename
  //
  static copyFile({ srcPath, destPath, success, fail, complete }) {
    wrapAsync(op_copy_file(srcPath, destPath), "copyFile:ok", "copyFile", { success, fail, complete });
  }

  static copyFileSync(srcPath_or_obj, destPath_arg) {
    let srcPath, destPath;
    if (typeof srcPath_or_obj === "object" && srcPath_or_obj !== null) {
      ({ srcPath, destPath } = srcPath_or_obj);
    } else {
      srcPath = srcPath_or_obj;
      destPath = destPath_arg;
    }
    wrapSync(() => op_copy_file_sync(srcPath, destPath), "copyFileSync");
  }

  static rename({ oldPath, newPath, success, fail, complete }) {
    wrapAsync(op_rename(oldPath, newPath), "rename:ok", "rename", { success, fail, complete });
  }

  static renameSync(oldPath_or_obj, newPath_arg) {
    let oldPath, newPath;
    if (typeof oldPath_or_obj === "object" && oldPath_or_obj !== null) {
      ({ oldPath, newPath } = oldPath_or_obj);
    } else {
      oldPath = oldPath_or_obj;
      newPath = newPath_arg;
    }
    wrapSync(() => op_rename_sync(oldPath, newPath), "renameSync");
  }

  //
  // fstat / ftruncate
  //
  static fstat({ fd, success, fail, complete }) {
    wrapAsync(
      op_fstat(ensureFd(fd)).then((stat) => ({ stats: new Stats(stat) })),
      "fstat:ok",
      "fstat",
      { success, fail, complete }
    );
  }

  static fstatSync({ fd }) {
    return wrapSync(() => ({ stats: new Stats(op_fstat_sync(ensureFd(fd))) }), "fstatSync");
  }

  static ftruncate({ fd, length = 0, success, fail, complete }) {
    const len = Number(length) || 0;
    wrapAsync(op_ftruncate(ensureFd(fd), len), "ftruncate:ok", "ftruncate", { success, fail, complete });
  }

  static ftruncateSync({ fd, length = 0 }) {
    const len = Number(length) || 0;
    wrapSync(() => op_ftruncate_sync(ensureFd(fd), len), "ftruncateSync");
  }

  //
  // mkdir / readdir
  //
  static mkdir({ dirPath, recursive = false, success, fail, complete }) {
    wrapAsync(op_mkdir(dirPath, !!recursive), "mkdir:ok", "mkdir", { success, fail, complete });
  }

  static mkdirSync(dirPath_or_obj, recursive_arg) {
    let dirPath, recursive;
    if (typeof dirPath_or_obj === "object" && dirPath_or_obj !== null) {
      ({ dirPath, recursive = false } = dirPath_or_obj);
    } else {
      dirPath = dirPath_or_obj;
      recursive = recursive_arg ?? false;
    }
    wrapSync(() => op_mkdir_sync(dirPath, !!recursive), "mkdirSync");
  }

  static readdir({ dirPath, success, fail, complete }) {
    wrapAsync(op_readdir(dirPath).then((files) => ({ files })), "readdir:ok", "readdir", {
      success, fail, complete,
    });
  }

  static readdirSync(dirPath_or_obj) {
    const dirPath = typeof dirPath_or_obj === "object" && dirPath_or_obj !== null ? dirPath_or_obj.dirPath : dirPath_or_obj;
    return wrapSync(() => ({ files: op_readdir_sync(dirPath) }), "readdirSync");
  }

  //
  // unlink / removeSavedFile / rmdir
  //
  static removeSavedFile({ filePath, success, fail, complete }) {
    wrapAsync(op_unlink(filePath), "removeSavedFile:ok", "removeSavedFile", { success, fail, complete });
  }

  static unlink({ filePath, success, fail, complete }) {
    wrapAsync(op_unlink(filePath), "unlink:ok", "unlink", { success, fail, complete });
  }

  static unlinkSync(filePath_or_obj) {
    const filePath = typeof filePath_or_obj === "object" && filePath_or_obj !== null ? filePath_or_obj.filePath : filePath_or_obj;
    return wrapSync(() => op_unlink_sync(filePath), "unlinkSync");
  }

  static rmdir({ dirPath, recursive = false, success, fail, complete }) {
    wrapAsync(op_rmdir(dirPath, !!recursive), "rmdir:ok", "rmdir", { success, fail, complete });
  }

  static rmdirSync(dirPath_or_obj, recursive_arg) {
    let dirPath, recursive;
    if (typeof dirPath_or_obj === "object" && dirPath_or_obj !== null) {
      ({ dirPath, recursive = false } = dirPath_or_obj);
    } else {
      dirPath = dirPath_or_obj;
      recursive = recursive_arg ?? false;
    }
    wrapSync(() => op_rmdir_sync(dirPath, !!recursive), "rmdirSync");
  }

  //
  // stat
  //
  static stat({ path, recursive = false, success, fail, complete }) {
    wrapAsync(
      op_stat(path, !!recursive).then((stat) => {
        if (!recursive) return new Stats(stat);
        return stat.map((item) => new FileStats(item.path, item.stat));
      }),
      "stat:ok",
      "stat",
      { success, fail, complete }
    );
  }

  static statSync(path_or_obj, recursive_arg) {
    let path, recursive;
    if (typeof path_or_obj === "object" && path_or_obj !== null) {
      ({ path, recursive = false } = path_or_obj);
    } else {
      path = path_or_obj;
      recursive = recursive_arg ?? false;
    }
    return wrapSync(() => {
      const stat = op_stat_sync(path, !!recursive);
      if (!recursive) return new Stats(stat);
      return stat.map((item) => new FileStats(item.path, item.stat));
    }, "statSync");
  }

  //
  // truncate (path) - ensure close always
  //
  static truncate({ filePath, length = 0, success, fail, complete }) {
    const len = Number(length) || 0;

    const p = op_open_file(filePath, "r+").then(async (fd) => {
      try {
        await op_ftruncate(fd, len);
      } finally {
        try { await op_close_file(fd); } catch (_) { }
      }
      return undefined;
    });

    wrapAsync(p, "truncate:ok", "truncate", { success, fail, complete });
  }

  static truncateSync({ filePath, length = 0 }) {
    const len = Number(length) || 0;
    return wrapSync(() => {
      let fd;
      try {
        fd = op_open_file_sync(filePath, "r+");
        op_ftruncate_sync(fd, len);
      } finally {
        if (fd !== undefined) {
          try { op_close_file_sync(fd); } catch (_) { }
        }
      }
      return undefined;
    }, "truncateSync");
  }

  //
  // write(fd)
  //
  static write({ fd, data, offset = 0, length, encoding = "utf8", position, success, fail, complete }) {
    let data_buf, data_str;

    try {
      ({ data_buf, data_str } = toUint8Array(data, offset, length));
    } catch (err) {
      const msg = `write:fail ${extractErrText(err)}`;
      const out = { errMsg: msg };
      fail?.(out);
      complete?.(out);
      return;
    }

    let pos = position;
    if (typeof pos !== "number" || !Number.isFinite(pos)) pos = undefined;

    const p = op_write_file(ensureFd(fd), data_buf, data_str, encoding, pos)
      .then((bytesWritten) => ({ bytesWritten }));

    wrapAsync(p, "write:ok", "write", { success, fail, complete });
  }

  static writeSync({ fd, data, offset = 0, length, encoding = "utf8", position }) {
    const { data_buf, data_str } = toUint8Array(data, offset, length);

    let pos = position;
    if (typeof pos !== "number" || !Number.isFinite(pos)) pos = undefined;

    return wrapSync(() => {
      const bytesWritten = op_write_file_sync(ensureFd(fd), data_buf, data_str, encoding, pos);
      return { bytesWritten };
    }, "writeSync");
  }

  //
  // readFile (path)
  //
  static readFile({ filePath, encoding, position, length, success, fail, complete }) {
    // Convert position/length to BigInt for native op (or undefined)
    const pos = typeof position === "number" && position >= 0 ? BigInt(position) : undefined;
    const len = typeof length === "number" && length > 0 ? BigInt(length) : undefined;

    const p = op_read_file(filePath, pos, len).then((data) => {
      // data is Uint8Array from native (already sliced by position/length)
      return BaseFileManager.#decodeReadData(data, encoding);
    });

    wrapAsync(p, "readFile:ok", "readFile", { success, fail, complete });
  }

  static readFileSync(filePath_or_obj, encoding_arg, position_arg, length_arg) {
    let filePath, encoding, position, length;
    if (typeof filePath_or_obj === "object" && filePath_or_obj !== null) {
      ({ filePath, encoding, position, length } = filePath_or_obj);
    } else {
      filePath = filePath_or_obj;
      encoding = encoding_arg;
      position = position_arg;
      length = length_arg;
    }
    // Convert position/length to BigInt for native op (or undefined)
    const pos = typeof position === "number" && position >= 0 ? BigInt(position) : undefined;
    const len = typeof length === "number" && length > 0 ? BigInt(length) : undefined;

    return wrapSync(() => {
      const data = op_read_file_sync(filePath, pos, len);
      return BaseFileManager.#decodeReadData(data, encoding);
    }, "readFileSync");
  }

  // Decode binary data based on encoding
  // If encoding is undefined, return ArrayBuffer
  // Otherwise decode to string with specified encoding
  static #decodeReadData(data, encoding) {
    const bytes = data;

    // No encoding specified - return ArrayBuffer
    if (encoding === undefined || encoding === null) {
      return { data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
    }

    // Decode based on encoding
    const enc = String(encoding).toLowerCase();
    let result;

    switch (enc) {
      case "utf8":
      case "utf-8":
        result = new TextDecoder("utf-8").decode(bytes);
        break;

      case "utf16le":
      case "utf-16le":
      case "ucs2":
      case "ucs-2":
        result = new TextDecoder("utf-16le").decode(bytes);
        break;

      case "latin1":
      case "binary":
        // Latin1: each byte maps to a character (0x00-0xFF)
        result = "";
        for (let i = 0; i < bytes.length; i++) {
          result += String.fromCharCode(bytes[i]);
        }
        break;

      case "hex":
        // Convert bytes to hex string
        result = "";
        for (let i = 0; i < bytes.length; i++) {
          result += bytes[i].toString(16).padStart(2, "0");
        }
        break;

      case "base64": {
        // Convert bytes to base64 string (chunked to avoid stack overflow on large files)
        let binary = "";
        for (let i = 0; i < bytes.length; i++) {
          binary += String.fromCharCode(bytes[i]);
        }
        result = btoa(binary);
        break;
      }

      case "ascii":
        // ASCII: mask to 7-bit
        result = "";
        for (let i = 0; i < bytes.length; i++) {
          result += String.fromCharCode(bytes[i] & 0x7F);
        }
        break;

      default:
        throw new IOError(`Unsupported encoding: ${encoding}`);
    }

    return { data: result };
  }

  //
  // placeholders
  //
  /**
   * Extract a zip file to target directory.
   * Default implementation uses IOCmd::Unzip (Rust zip crate on IO thread).
   * Platforms can override this (e.g. Android uses java.util.zip via JNI).
   */
  static unzip({ zipFilePath, targetPath, success, fail, complete }) {
    wrapAsync(
      op_unzip(zipFilePath, targetPath),
      "unzip:ok",
      "unzip",
      { success, fail, complete }
    );
  }
  static read({ fd, arrayBuffer, offset = 0, length = 0, position, success, fail, complete }) {
    const numFd = ensureFd(fd);
    const readLen = length > 0 ? length : (arrayBuffer ? arrayBuffer.byteLength - offset : 0);
    if (readLen <= 0) {
      const out = { errMsg: "read:fail invalid length" };
      fail?.(out);
      complete?.(out);
      return;
    }
    let pos;
    if (typeof position === "number" && Number.isFinite(position) && position >= 0) {
      pos = BigInt(position);
    }

    const p = op_read_fd(numFd, BigInt(readLen), pos).then((data) => {
      const src = new Uint8Array(data.buffer || data);
      const dst = new Uint8Array(arrayBuffer);
      dst.set(src.subarray(0, Math.min(src.length, dst.length - offset)), offset);
      return { bytesRead: src.length, arrayBuffer };
    });

    wrapAsync(p, "read:ok", "read", { success, fail, complete });
  }

  static readSync({ fd, arrayBuffer, offset = 0, length = 0, position }) {
    const numFd = ensureFd(fd);
    const readLen = length > 0 ? length : (arrayBuffer ? arrayBuffer.byteLength - offset : 0);
    if (readLen <= 0) throw { errMsg: "readSync:fail invalid length" };
    let pos;
    if (typeof position === "number" && Number.isFinite(position) && position >= 0) {
      pos = BigInt(position);
    }

    return wrapSync(() => {
      const data = op_read_fd_sync(numFd, BigInt(readLen), pos);
      const src = new Uint8Array(data.buffer || data);
      const dst = new Uint8Array(arrayBuffer);
      dst.set(src.subarray(0, Math.min(src.length, dst.length - offset)), offset);
      return { bytesRead: src.length, arrayBuffer };
    }, "readSync");
  }
  static readCompressedFile({ filePath, compressionAlgorithm, success, fail, complete }) {
    if (compressionAlgorithm !== "br") {
      const out = { errMsg: "readCompressedFile:fail unsupported compressionAlgorithm" };
      fail?.(out);
      complete?.(out);
      return;
    }
    const p = op_read_compressed_file(filePath).then((data) => {
      const bytes = new Uint8Array(data.buffer || data);
      return { data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
    });
    wrapAsync(p, "readCompressedFile:ok", "readCompressedFile", { success, fail, complete });
  }

  static readCompressedFileSync(obj) {
    const filePath = obj.filePath;
    const compressionAlgorithm = obj.compressionAlgorithm;
    if (compressionAlgorithm !== "br") {
      throw { errMsg: "readCompressedFileSync:fail unsupported compressionAlgorithm" };
    }
    return wrapSync(() => {
      const data = op_read_compressed_file_sync(filePath);
      const bytes = new Uint8Array(data.buffer || data);
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    }, "readCompressedFileSync");
  }
  static readZipEntry({ filePath, encoding, entries, success, fail, complete }) {
    const entriesJson = JSON.stringify({ encoding, entries });
    const p = op_read_zip_entry(filePath, entriesJson).then((resultJson) => {
      const result = JSON.parse(resultJson);
      // For entries without encoding, decode base64 back to ArrayBuffer
      if (result.entries) {
        const keys = Object.keys(result.entries);
        for (let i = 0; i < keys.length; i++) {
          const item = result.entries[keys[i]];
          if (item.data !== null && item.data !== undefined && !encoding) {
            // Check per-entry encoding
            const entryDef = Array.isArray(entries) && entries.find(e => e.path === keys[i]);
            if (!entryDef || !entryDef.encoding) {
              // No encoding specified - data is base64, convert to ArrayBuffer
              const binary = atob(item.data);
              const buf = new ArrayBuffer(binary.length);
              const view = new Uint8Array(buf);
              for (let j = 0; j < binary.length; j++) {
                view[j] = binary.charCodeAt(j);
              }
              item.data = buf;
            }
          }
        }
      }
      return result;
    });
    wrapAsync(p, "readZipEntry:ok", "readZipEntry", { success, fail, complete });
  }

  static saveFile({ tempFilePath, filePath, success, fail, complete }) {
    if (typeof tempFilePath !== "string" || tempFilePath.length === 0) {
      const out = { errMsg: "saveFile:fail tempFilePath is required" };
      fail?.(out);
      complete?.(out);
      return;
    }

    const savedFilePath = resolveSavedFilePath(filePath, tempFilePath);
    const p = op_rename(tempFilePath, savedFilePath)
      .catch(async () => {
        await op_copy_file(tempFilePath, savedFilePath);
        await op_unlink(tempFilePath);
      })
      .then(() => ({ savedFilePath }));

    wrapAsync(p, "saveFile:ok", "saveFile", { success, fail, complete });
  }

  static saveFileSync(tempFilePath_or_obj, filePath_arg) {
    let tempFilePath, filePath;
    if (typeof tempFilePath_or_obj === "object" && tempFilePath_or_obj !== null) {
      ({ tempFilePath, filePath } = tempFilePath_or_obj);
    } else {
      tempFilePath = tempFilePath_or_obj;
      filePath = filePath_arg;
    }
    if (typeof tempFilePath !== "string" || tempFilePath.length === 0) {
      throw { errMsg: "saveFileSync:fail tempFilePath is required" };
    }

    return wrapSync(() => {
      const savedFilePath = resolveSavedFilePath(filePath, tempFilePath);
      try {
        op_rename_sync(tempFilePath, savedFilePath);
      } catch (_) {
        op_copy_file_sync(tempFilePath, savedFilePath);
        op_unlink_sync(tempFilePath);
      }
      return { savedFilePath };
    }, "saveFileSync");
  }
}

const fileSystemManager = BaseFileManager;

function getFileSystemManager() {
  return fileSystemManager;
}

export { BaseFileManager, getFileSystemManager };
