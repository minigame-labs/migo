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
  op_get_file_info, op_get_file_info_sync,
  op_list_saved_files,
  op_decode_multi_formats
} from "ext:core/ops";

import { core, primordials } from "ext:core/mod.js";
import { wrapAsync } from "ext:host_v8_base/02_async.js";
import { FileStats, Stats } from "./02_file_stats.js";

const { Error } = primordials;

class IOError extends Error {
  constructor(msg) {
    super(msg);
    this.name = "IOError";
  }
}
core.registerErrorClass("IOError", IOError);

function wrapSync(fn, failPrefix) {
  try {
    return fn();
  } catch (err) {
    const msg = err?.message || String(err);
    throw { errMsg: `${failPrefix}:fail ${msg}`, message: `${failPrefix}:fail ${msg}` };
  }
}

function toUint8Array(data, offset = 0, length) {
  if (typeof data === "string" || data instanceof String) {
    return { data_str: String(data), data_buf: null };
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

// Pre-computed hex lookup table (avoids toString(16) + padStart per byte)
const HEX_LUT = new Array(256);
for (let i = 0; i < 256; i++) HEX_LUT[i] = (i < 16 ? "0" : "") + i.toString(16);

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
  static access(options) {
    return wrapAsync('access', () => op_access(options.path), options);
  }

  static accessSync(path_or_obj) {
    const path = typeof path_or_obj === "object" && path_or_obj !== null ? path_or_obj.path : path_or_obj;
    wrapSync(() => { op_access_sync(path); }, "accessSync");
  }

  //
  // writeFile / appendFile (path)
  //
  static writeFile(options) {
    return BaseFileManager.#writeFileCommon(options, false);
  }

  static appendFile(options) {
    return BaseFileManager.#writeFileCommon(options, true);
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

  static #writeFileCommon(options, append) {
    const prefix = append ? "appendFile" : "writeFile";
    return wrapAsync(prefix, () => {
      const { data_buf, data_str } = toUint8Array(options.data);
      const encoding = options.encoding || "utf8";
      return op_write_or_append_file(options.filePath, data_buf, data_str, encoding, append)
        .then((ok) => {
          if (!ok) throw new IOError("unknown error");
        });
    }, options);
  }

  //
  // open / close
  //
  static open(options) {
    const flag = options.flag || "r";
    return wrapAsync('open', () => {
      return op_open_file(options.filePath, flag).then((fd) => ({ fd: String(fd) }));
    }, options);
  }

  static openSync({ filePath, flag = "r" }) {
    return wrapSync(() => String(op_open_file_sync(filePath, flag)), "openSync");
  }

  static close(options) {
    return wrapAsync('close', () => op_close_file(ensureFd(options.fd)), options);
  }

  static closeSync({ fd }) {
    wrapSync(() => op_close_file_sync(ensureFd(fd)), "closeSync");
  }

  //
  // copy / rename
  //
  static copyFile(options) {
    return wrapAsync('copyFile', () => op_copy_file(options.srcPath, options.destPath), options);
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

  static rename(options) {
    return wrapAsync('rename', () => op_rename(options.oldPath, options.newPath), options);
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
  static fstat(options) {
    return wrapAsync('fstat', () => {
      return op_fstat(ensureFd(options.fd)).then((stat) => ({ stats: new Stats(stat) }));
    }, options);
  }

  static fstatSync({ fd }) {
    return wrapSync(() => new Stats(op_fstat_sync(ensureFd(fd))), "fstatSync");
  }

  static ftruncate(options) {
    const len = Number(options.length) || 0;
    return wrapAsync('ftruncate', () => op_ftruncate(ensureFd(options.fd), len), options);
  }

  static ftruncateSync({ fd, length = 0 }) {
    const len = Number(length) || 0;
    wrapSync(() => op_ftruncate_sync(ensureFd(fd), len), "ftruncateSync");
  }

  //
  // mkdir / readdir
  //
  static mkdir(options) {
    return wrapAsync('mkdir', () => op_mkdir(options.dirPath, !!options.recursive), options);
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

  static readdir(options) {
    return wrapAsync('readdir', () => {
      return op_readdir(options.dirPath).then((files) => ({ files }));
    }, options);
  }

  static readdirSync(dirPath_or_obj) {
    const dirPath = typeof dirPath_or_obj === "object" && dirPath_or_obj !== null ? dirPath_or_obj.dirPath : dirPath_or_obj;
    return wrapSync(() => op_readdir_sync(dirPath), "readdirSync");
  }

  //
  // unlink / removeSavedFile / rmdir
  //
  static removeSavedFile(options) {
    return wrapAsync('removeSavedFile', () => op_unlink(options.filePath), options);
  }

  static unlink(options) {
    return wrapAsync('unlink', () => op_unlink(options.filePath), options);
  }

  static unlinkSync(filePath_or_obj) {
    const filePath = typeof filePath_or_obj === "object" && filePath_or_obj !== null ? filePath_or_obj.filePath : filePath_or_obj;
    return wrapSync(() => op_unlink_sync(filePath), "unlinkSync");
  }

  static rmdir(options) {
    return wrapAsync('rmdir', () => op_rmdir(options.dirPath, !!options.recursive), options);
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
  static stat(options) {
    const recursive = !!options.recursive;
    return wrapAsync('stat', () => {
      return op_stat(options.path, recursive).then((stat) => {
        if (!recursive) return { stats: new Stats(stat) };
        return { stats: stat.map((item) => new FileStats(item.path, item.stat)) };
      });
    }, options);
  }

  static statSync(path_or_obj, recursive_arg) {
    let path, recursive;
    if (typeof path_or_obj === "object" && path_or_obj !== null) {
      ({ path, recursive = false } = path_or_obj);
    } else {
      path = path_or_obj;
      recursive = recursive_arg ?? false;
    }
    const rec = !!recursive;
    return wrapSync(() => {
      const raw = op_stat_sync(path, rec);
      if (!rec) return new Stats(raw);
      return raw.map((item) => new FileStats(item.path, item.stat));
    }, "statSync");
  }

  //
  // truncate (path) - ensure close always
  //
  static truncate(options) {
    const len = Number(options.length) || 0;
    return wrapAsync('truncate', () => {
      return op_open_file(options.filePath, "r+").then(async (fd) => {
        try {
          await op_ftruncate(fd, len);
        } finally {
          try { await op_close_file(fd); } catch (_) { }
        }
      });
    }, options);
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
  static write(options) {
    return wrapAsync('write', () => {
      const { data_buf, data_str } = toUint8Array(options.data, options.offset || 0, options.length);
      const encoding = options.encoding || "utf8";
      let pos = options.position;
      if (typeof pos !== "number" || !Number.isFinite(pos)) pos = undefined;
      return op_write_file(ensureFd(options.fd), data_buf, data_str, encoding, pos)
        .then((bytesWritten) => ({ bytesWritten }));
    }, options);
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
  static readFile(options) {
    const encoding = options.encoding;
    const pos = typeof options.position === "number" && Number.isFinite(options.position) && options.position >= 0
      ? BigInt(Math.trunc(options.position))
      : undefined;
    const len = typeof options.length === "number" && Number.isFinite(options.length) && options.length > 0
      ? BigInt(Math.trunc(options.length))
      : undefined;

    return wrapAsync('readFile', () => {
      return op_read_file(options.filePath, pos, len).then((data) => {
        return BaseFileManager.#decodeReadData(data, encoding);
      });
    }, options);
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
    const pos = typeof position === "number" && Number.isFinite(position) && position >= 0
      ? BigInt(Math.trunc(position))
      : undefined;
    const len = typeof length === "number" && Number.isFinite(length) && length > 0
      ? BigInt(Math.trunc(length))
      : undefined;

    return wrapSync(() => {
      const raw = op_read_file_sync(filePath, pos, len);
      return BaseFileManager.#decodeReadResult(raw, encoding);
    }, "readFileSync");
  }

  static #decodeReadResult(bytes, encoding) {
    if (encoding === undefined || encoding === null || encoding === "") {
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    }
    return BaseFileManager.#decodeBytes(bytes, encoding);
  }

  static #decodeReadData(bytes, encoding) {
    return { data: BaseFileManager.#decodeReadResult(bytes, encoding) };
  }

  static #decodeBytes(bytes, encoding) {
    const enc = String(encoding).toLowerCase();
    const len = bytes.length;

    switch (enc) {
      case "utf8":
      case "utf-8":
        return op_decode_multi_formats(bytes, "utf-8");

      case "utf16le":
      case "utf-16le":
      case "ucs2":
      case "ucs-2":
        return op_decode_multi_formats(bytes, "utf-16le");

      case "latin1":
      case "binary":
        return BaseFileManager.#bytesToStringChunked(bytes, len, false);

      case "ascii":
        return BaseFileManager.#bytesToStringChunked(bytes, len, true);

      case "hex": {
        const parts = new Array(len);
        for (let i = 0; i < len; i++) {
          parts[i] = HEX_LUT[bytes[i]];
        }
        return parts.join("");
      }

      case "base64":
        return btoa(BaseFileManager.#bytesToStringChunked(bytes, len, false));

      default:
        throw new IOError(`Unsupported encoding: ${encoding}`);
    }
  }

  static #bytesToStringChunked(bytes, len, ascii) {
    const CHUNK = 8192;
    if (len <= CHUNK) {
      return ascii
        ? String.fromCharCode.apply(null, BaseFileManager.#maskAscii(bytes, 0, len))
        : String.fromCharCode.apply(null, bytes);
    }
    const parts = [];
    for (let i = 0; i < len; i += CHUNK) {
      const end = i + CHUNK < len ? i + CHUNK : len;
      const slice = bytes.subarray(i, end);
      parts.push(
        ascii
          ? String.fromCharCode.apply(null, BaseFileManager.#maskAscii(slice, 0, slice.length))
          : String.fromCharCode.apply(null, slice)
      );
    }
    return parts.join("");
  }

  static #maskAscii(bytes, start, end) {
    const out = new Uint8Array(end - start);
    for (let i = start; i < end; i++) out[i - start] = bytes[i] & 0x7F;
    return out;
  }

  //
  // unzip
  //
  static unzip(options) {
    return wrapAsync('unzip', () => op_unzip(options.zipFilePath, options.targetPath), options);
  }

  //
  // read(fd)
  //
  static read(options) {
    return wrapAsync('read', () => {
      const numFd = ensureFd(options.fd);
      const arrayBuffer = options.arrayBuffer;
      if (!arrayBuffer || !(arrayBuffer instanceof ArrayBuffer)) {
        throw new IOError("arrayBuffer must be an ArrayBuffer instance");
      }
      let offset = Math.trunc(options.offset || 0);
      if (offset < 0 || offset >= arrayBuffer.byteLength) {
        throw new IOError("invalid offset");
      }
      const maxLen = arrayBuffer.byteLength - offset;
      const length = options.length || 0;
      let readLen = length > 0 ? Math.min(Math.trunc(length), maxLen) : maxLen;
      if (readLen <= 0) throw new IOError("invalid length");

      let pos;
      if (typeof options.position === "number" && Number.isFinite(options.position) && options.position >= 0) {
        pos = BigInt(Math.trunc(options.position));
      }

      return op_read_fd(numFd, BigInt(readLen), pos).then((data) => {
        const src = new Uint8Array(data.buffer || data);
        const dst = new Uint8Array(arrayBuffer);
        const bytesRead = Math.min(src.length, maxLen);
        dst.set(src.subarray(0, bytesRead), offset);
        return { bytesRead, arrayBuffer };
      });
    }, options);
  }

  static readSync({ fd, arrayBuffer, offset = 0, length = 0, position }) {
    const numFd = ensureFd(fd);
    if (!arrayBuffer || !(arrayBuffer instanceof ArrayBuffer)) {
      throw new IOError("readSync: arrayBuffer must be an ArrayBuffer instance");
    }
    offset = Math.trunc(offset);
    if (offset < 0 || offset >= arrayBuffer.byteLength) {
      throw new IOError("readSync: invalid offset");
    }
    const maxLen = arrayBuffer.byteLength - offset;
    let readLen = length > 0 ? Math.min(Math.trunc(length), maxLen) : maxLen;
    if (readLen <= 0) throw new IOError("readSync: invalid length");
    let pos;
    if (typeof position === "number" && Number.isFinite(position) && position >= 0) {
      pos = BigInt(Math.trunc(position));
    }

    return wrapSync(() => {
      const data = op_read_fd_sync(numFd, BigInt(readLen), pos);
      const src = new Uint8Array(data.buffer || data);
      const dst = new Uint8Array(arrayBuffer);
      const bytesRead = Math.min(src.length, maxLen);
      dst.set(src.subarray(0, bytesRead), offset);
      return { bytesRead, arrayBuffer };
    }, "readSync");
  }

  //
  // readCompressedFile
  //
  static readCompressedFile(options) {
    return wrapAsync('readCompressedFile', () => {
      if (options.compressionAlgorithm !== "br") {
        throw new IOError("unsupported compressionAlgorithm");
      }
      return op_read_compressed_file(options.filePath).then((data) => {
        const bytes = new Uint8Array(data.buffer || data);
        return { data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
      });
    }, options);
  }

  static readCompressedFileSync(obj) {
    const filePath = obj.filePath;
    const compressionAlgorithm = obj.compressionAlgorithm;
    if (compressionAlgorithm !== "br") {
      throw new IOError("readCompressedFileSync: unsupported compressionAlgorithm");
    }
    return wrapSync(() => {
      const data = op_read_compressed_file_sync(filePath);
      const bytes = new Uint8Array(data.buffer || data);
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    }, "readCompressedFileSync");
  }

  //
  // readZipEntry
  //
  static readZipEntry(options) {
    const encoding = options.encoding;
    const entries = options.entries;
    const entriesJson = JSON.stringify({ encoding, entries });
    let encodingMap;
    if (!encoding && Array.isArray(entries)) {
      encodingMap = new Map();
      for (let i = 0; i < entries.length; i++) {
        if (entries[i].encoding) encodingMap.set(entries[i].path, entries[i].encoding);
      }
    }
    return wrapAsync('readZipEntry', () => {
      return op_read_zip_entry(options.filePath, entriesJson).then((result) => {
        if (result.entries && !encoding) {
          const keys = Object.keys(result.entries);
          for (let i = 0; i < keys.length; i++) {
            const item = result.entries[keys[i]];
            if (item.data !== null && item.data !== undefined) {
              if (!encodingMap || !encodingMap.has(keys[i])) {
                const binary = atob(item.data);
                const len = binary.length;
                const buf = new ArrayBuffer(len);
                const view = new Uint8Array(buf);
                for (let j = 0; j < len; j++) {
                  view[j] = binary.charCodeAt(j);
                }
                item.data = buf;
              }
            }
          }
        }
        return result;
      });
    }, options);
  }

  //
  // saveFile
  //
  static saveFile(options) {
    return wrapAsync('saveFile', () => {
      const tempFilePath = options.tempFilePath;
      if (typeof tempFilePath !== "string" || tempFilePath.length === 0) {
        throw new IOError("tempFilePath is required");
      }
      const savedFilePath = resolveSavedFilePath(options.filePath, tempFilePath);
      return op_rename(tempFilePath, savedFilePath)
        .catch(async () => {
          await op_copy_file(tempFilePath, savedFilePath);
          await op_unlink(tempFilePath);
        })
        .then(() => ({ savedFilePath }));
    }, options);
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
      throw new IOError("saveFileSync: tempFilePath is required");
    }

    return wrapSync(() => {
      const savedFilePath = resolveSavedFilePath(filePath, tempFilePath);
      try {
        op_rename_sync(tempFilePath, savedFilePath);
      } catch (_) {
        op_copy_file_sync(tempFilePath, savedFilePath);
        op_unlink_sync(tempFilePath);
      }
      return savedFilePath;
    }, "saveFileSync");
  }

  //
  // getFileInfo
  //
  static getFileInfo(options) {
    return wrapAsync('getFileInfo', () => {
      return op_get_file_info(options.filePath, options.digestAlgorithm || "md5")
        .then(([size, digest]) => ({ size, digest }));
    }, options);
  }

  //
  // getSavedFileList
  //
  static getSavedFileList(options) {
    return wrapAsync('getSavedFileList', () => {
      return op_list_saved_files(SAVE_FILE_DIR, "saved_")
        .then((fileList) => ({ fileList }));
    }, options || {});
  }
}

const fileSystemManager = BaseFileManager;

function getFileSystemManager() {
  return fileSystemManager;
}

export { BaseFileManager, getFileSystemManager };
