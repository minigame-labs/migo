// WebSocket — wraps migo.connectSocket. Browsers expose readyState
// CONNECTING/OPEN/CLOSING/CLOSED, send/close, onopen/onmessage/onclose/onerror.

import EventTarget from "./event-target.js";

const CONNECTING = 0, OPEN = 1, CLOSING = 2, CLOSED = 3;

export default class WebSocket extends EventTarget {
  constructor(url, protocols) {
    super();
    if (typeof migo.connectSocket !== "function") {
      throw new Error("[migo-adapter] migo.connectSocket is not available");
    }
    this.url = url;
    this.protocol = Array.isArray(protocols) ? protocols.join(",") : (protocols || "");
    this.readyState = CONNECTING;
    this.binaryType = "arraybuffer";
    this.onopen = null;
    this.onmessage = null;
    this.onclose = null;
    this.onerror = null;

    this._task = migo.connectSocket({ url, protocols: protocols && (Array.isArray(protocols) ? protocols : [protocols]) });

    this._task.onOpen && this._task.onOpen((res) => {
      this.readyState = OPEN;
      const ev = { type: "open", target: this };
      if (this.onopen) try { this.onopen(ev); } catch {}
      this.dispatchEvent(ev);
    });
    this._task.onMessage && this._task.onMessage((res) => {
      const ev = { type: "message", data: res.data, target: this };
      if (this.onmessage) try { this.onmessage(ev); } catch {}
      this.dispatchEvent(ev);
    });
    this._task.onClose && this._task.onClose((res) => {
      this.readyState = CLOSED;
      const ev = { type: "close", code: res && res.code, reason: res && res.reason, target: this };
      if (this.onclose) try { this.onclose(ev); } catch {}
      this.dispatchEvent(ev);
    });
    this._task.onError && this._task.onError((err) => {
      const ev = { type: "error", error: err, target: this };
      if (this.onerror) try { this.onerror(ev); } catch {}
      this.dispatchEvent(ev);
    });
  }

  send(data) {
    if (this.readyState !== OPEN) {
      throw new Error("WebSocket is not open: readyState " + this.readyState);
    }
    this._task.send({ data });
  }

  close(code, reason) {
    if (this.readyState === CLOSED || this.readyState === CLOSING) return;
    this.readyState = CLOSING;
    this._task.close({ code, reason });
  }
}
WebSocket.CONNECTING = CONNECTING;
WebSocket.OPEN = OPEN;
WebSocket.CLOSING = CLOSING;
WebSocket.CLOSED = CLOSED;
