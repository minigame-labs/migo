// XMLHttpRequest — wraps migo.request. Engines use this for resource loading
// (JSON/atlases/binary). Minimum viable surface: open/setRequestHeader/send,
// status/responseText/response/responseType, onreadystatechange/onload/onerror,
// readyState transitions.

import EventTarget from "./event-target.js";

const UNSENT = 0, OPENED = 1, HEADERS_RECEIVED = 2, LOADING = 3, DONE = 4;

export default class XMLHttpRequest extends EventTarget {
  constructor() {
    super();
    this._method = "GET";
    this._url = "";
    this._headers = {};
    this._task = null;

    this.readyState = UNSENT;
    this.status = 0;
    this.statusText = "";
    this.response = null;
    this.responseText = "";
    this.responseType = ""; // "" | "text" | "arraybuffer" | "json"
    this.timeout = 0;
    this.withCredentials = false;
    this.onreadystatechange = null;
    this.onload = null;
    this.onerror = null;
    this.ontimeout = null;
    this.onabort = null;
  }

  open(method, url /* , async, user, password */) {
    this._method = String(method).toUpperCase();
    this._url = url;
    this._setReady(OPENED);
  }

  setRequestHeader(name, value) { this._headers[name] = value; }
  getResponseHeader() { return null; }
  getAllResponseHeaders() { return ""; }

  abort() {
    if (this._task && typeof this._task.abort === "function") this._task.abort();
    if (this.onabort) try { this.onabort({ type: "abort" }); } catch {}
  }

  send(body) {
    if (typeof migo.request !== "function") {
      throw new Error("[migo-adapter] migo.request is not available");
    }
    const dataType = this.responseType === "json" ? "json" : undefined;
    // Use the host's arraybuffer mode when caller wants binary; otherwise text.
    const responseType = (this.responseType === "arraybuffer") ? "arraybuffer" : "text";

    this._task = migo.request({
      url: this._url,
      method: this._method,
      header: this._headers,
      data: body,
      dataType,
      responseType,
      success: (res) => {
        this.status = res.statusCode || 200;
        this.statusText = String(this.status);
        this._setReady(HEADERS_RECEIVED);
        this._setReady(LOADING);
        if (this.responseType === "arraybuffer") {
          this.response = res.data;
          this.responseText = "";
        } else if (this.responseType === "json") {
          this.response = res.data;
          this.responseText = typeof res.data === "string" ? res.data : JSON.stringify(res.data);
        } else {
          this.responseText = typeof res.data === "string" ? res.data : JSON.stringify(res.data);
          this.response = this.responseText;
        }
        this._setReady(DONE);
        if (this.onload) try { this.onload({ type: "load", target: this }); } catch {}
      },
      fail: (err) => {
        this.status = 0;
        this._setReady(DONE);
        if (this.onerror) try { this.onerror({ type: "error", error: err }); } catch {}
      },
    });
  }

  _setReady(s) {
    this.readyState = s;
    if (this.onreadystatechange) try { this.onreadystatechange({ type: "readystatechange" }); } catch {}
  }
}
XMLHttpRequest.UNSENT = UNSENT;
XMLHttpRequest.OPENED = OPENED;
XMLHttpRequest.HEADERS_RECEIVED = HEADERS_RECEIVED;
XMLHttpRequest.LOADING = LOADING;
XMLHttpRequest.DONE = DONE;
