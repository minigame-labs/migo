// FileReader — reads a Blob/ArrayBuffer/File-ish thing. Engines hit this
// rarely (some image upload paths). We support readAsText / readAsArrayBuffer
// / readAsDataURL on plain ArrayBuffer or string input. Anything fancier
// (real Blob streaming) is not in scope.

import EventTarget from "./event-target.js";

const EMPTY = 0, LOADING = 1, DONE = 2;

export default class FileReader extends EventTarget {
  constructor() {
    super();
    this.readyState = EMPTY;
    this.result = null;
    this.error = null;
    this.onloadstart = null;
    this.onprogress = null;
    this.onload = null;
    this.onloadend = null;
    this.onerror = null;
    this.onabort = null;
  }

  abort() {
    this.readyState = DONE;
    if (this.onabort) try { this.onabort({ type: "abort" }); } catch {}
  }

  readAsText(blob, _encoding) {
    this._read(blob, "text");
  }
  readAsArrayBuffer(blob) {
    this._read(blob, "arraybuffer");
  }
  readAsDataURL(blob) {
    this._read(blob, "dataurl");
  }

  _read(blob, mode) {
    this.readyState = LOADING;
    Promise.resolve().then(() => {
      try {
        if (mode === "text") {
          if (typeof blob === "string") this.result = blob;
          else if (blob instanceof ArrayBuffer) this.result = new TextDecoder().decode(blob);
          else this.result = String(blob);
        } else if (mode === "arraybuffer") {
          if (blob instanceof ArrayBuffer) this.result = blob;
          else if (typeof blob === "string") this.result = new TextEncoder().encode(blob).buffer;
          else this.result = blob;
        } else if (mode === "dataurl") {
          // No host base64 helper assumed — produce a minimal placeholder.
          this.result = "data:application/octet-stream;base64,";
        }
        this.readyState = DONE;
        if (this.onload) try { this.onload({ type: "load", target: this }); } catch {}
        if (this.onloadend) try { this.onloadend({ type: "loadend", target: this }); } catch {}
      } catch (e) {
        this.readyState = DONE;
        this.error = e;
        if (this.onerror) try { this.onerror({ type: "error", error: e }); } catch {}
      }
    });
  }
}
FileReader.EMPTY = EMPTY;
FileReader.LOADING = LOADING;
FileReader.DONE = DONE;
