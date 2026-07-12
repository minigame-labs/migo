/* @minigame-labs/migo-adapter — IIFE bundle. Source: adapter/src/index.js */
(() => {
  // src/bom.js
  var _info = (() => {
    try {
      if (typeof migo.getWindowInfo === "function") return migo.getWindowInfo();
    } catch {
    }
    try {
      if (typeof migo.getSystemInfoSync === "function") return migo.getSystemInfoSync();
    } catch {
    }
    return { windowWidth: 0, windowHeight: 0, screenWidth: 0, screenHeight: 0, pixelRatio: 1 };
  })();
  var _dpr = _info.pixelRatio || 1;
  var devicePixelRatio = 1;
  var innerWidth = Math.round((_info.windowWidth || _info.screenWidth || 0) * _dpr);
  var innerHeight = Math.round((_info.windowHeight || _info.screenHeight || 0) * _dpr);
  var outerWidth = innerWidth;
  var outerHeight = innerHeight;
  var screenWidth = Math.round((_info.screenWidth || _info.windowWidth || 0) * _dpr);
  var screenHeight = Math.round((_info.screenHeight || _info.windowHeight || 0) * _dpr);
  var screen = {
    width: screenWidth,
    height: screenHeight,
    availWidth: innerWidth,
    availHeight: innerHeight,
    availLeft: 0,
    availTop: 0,
    orientation: { angle: 0, type: innerWidth > innerHeight ? "landscape-primary" : "portrait-primary" }
  };

  // src/navigator.js
  var _info2 = {};
  try {
    if (typeof migo.getSystemInfoSync === "function") _info2 = migo.getSystemInfoSync() || {};
  } catch {
  }
  var platform = (_info2.platform || "ios").toLowerCase();
  var system = _info2.system || "";
  var version = _info2.version || "1.0.0";
  var navigator = {
    platform,
    language: _info2.language || "zh-CN",
    languages: [_info2.language || "zh-CN"],
    appVersion: `5.0 (${platform === "android" ? `Linux; ${system}` : `iPhone; ${system}`})`,
    userAgent: `Mozilla/5.0 (${platform === "android" ? "Linux; Android" : "iPhone; iOS"}; ${system}) AppleWebKit/605.1.15 (KHTML, like Gecko) MiniGame/${version} migo`,
    vendor: "",
    product: "Gecko",
    productSub: "20030107",
    cookieEnabled: false,
    onLine: true,
    hardwareConcurrency: 4,
    maxTouchPoints: 5,
    // Stubs for APIs the runtime doesn't bridge yet.
    geolocation: {
      getCurrentPosition: () => {
      },
      watchPosition: () => 0,
      clearWatch: () => {
      }
    }
  };
  var navigator_default = navigator;

  // src/location.js
  var location = {
    href: "game.js",
    protocol: "https:",
    host: "game",
    hostname: "game",
    port: "",
    pathname: "/game.js",
    search: "",
    hash: "",
    origin: "https://game",
    reload() {
    },
    replace() {
    },
    assign() {
    },
    toString() {
      return this.href;
    }
  };
  var location_default = location;

  // src/event-target.js
  var EventTarget = class {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, listener) {
      if (typeof listener !== "function") return;
      const list = this._listeners[type] || (this._listeners[type] = []);
      if (list.indexOf(listener) === -1) list.push(listener);
    }
    removeEventListener(type, listener) {
      const list = this._listeners[type];
      if (!list) return;
      const i = list.indexOf(listener);
      if (i !== -1) list.splice(i, 1);
    }
    dispatchEvent(event) {
      if (!event || !event.type) return false;
      const list = this._listeners[event.type];
      if (!list || list.length === 0) return true;
      if (event.target == null) event.target = this;
      if (event.currentTarget == null) event.currentTarget = this;
      const snapshot = list.slice();
      for (let i = 0; i < snapshot.length; i++) {
        try {
          snapshot[i].call(this, event);
        } catch (e) {
          if (typeof console !== "undefined" && console.error) console.error(e);
        }
      }
      return !event.defaultPrevented;
    }
  };

  // src/element.js
  var Node = class extends EventTarget {
    constructor() {
      super();
      this.children = [];
      this.childNodes = this.children;
      this.parentNode = null;
      this.ownerDocument = null;
    }
    appendChild(node) {
      if (!node) return null;
      if (node.parentNode) node.parentNode.removeChild(node);
      this.children.push(node);
      node.parentNode = this;
      return node;
    }
    removeChild(node) {
      const i = this.children.indexOf(node);
      if (i !== -1) {
        this.children.splice(i, 1);
        node.parentNode = null;
      }
      return node;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.children.indexOf(refNode);
      if (i === -1) return this.appendChild(newNode);
      if (newNode.parentNode) newNode.parentNode.removeChild(newNode);
      this.children.splice(i, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    cloneNode() {
      return null;
    }
    // `node.contains(other)` — engines (e.g. PixiJS CanvasSource) call
    // `document.body.contains(canvas)` to decide if a canvas is live in the DOM.
    contains(node) {
      if (node == null) return false;
      if (node === this) return true;
      for (const c of this.children) {
        if (c === node || typeof c.contains === "function" && c.contains(node)) return true;
      }
      return false;
    }
  };
  var Element = class extends Node {
    constructor() {
      super();
      this.style = {};
      this.classList = [];
      this.className = "";
      this.id = "";
      this.dataset = {};
      this.clientLeft = 0;
      this.clientTop = 0;
      this.scrollLeft = 0;
      this.scrollTop = 0;
    }
  };
  var HTMLElement = class extends Element {
    constructor(tagName = "") {
      super();
      this.tagName = String(tagName).toUpperCase();
      this.nodeName = this.tagName;
      this.innerHTML = "";
    }
    get clientWidth() {
      return globalThis.innerWidth || 0;
    }
    get clientHeight() {
      return globalThis.innerHeight || 0;
    }
    get offsetWidth() {
      return this.clientWidth;
    }
    get offsetHeight() {
      return this.clientHeight;
    }
    setAttribute(name, value) {
      this[name] = value;
    }
    getAttribute(name) {
      return this[name] == null ? null : this[name];
    }
    removeAttribute(name) {
      delete this[name];
    }
    hasAttribute(name) {
      return name in this;
    }
    getBoundingClientRect() {
      const w = this.clientWidth, h = this.clientHeight;
      return { top: 0, left: 0, right: w, bottom: h, width: w, height: h, x: 0, y: 0 };
    }
    focus() {
    }
    blur() {
    }
    click() {
    }
  };
  var HTMLImageElement = class extends HTMLElement {
    constructor() {
      super("img");
    }
  };
  var HTMLCanvasElement = class _HTMLCanvasElement extends HTMLElement {
    constructor() {
      super("canvas");
    }
    // `document.createElement('canvas')` / `new Canvas()` returns the native
    // `migo.createCanvas()` object, which is NOT in this class hierarchy. Engines
    // detect canvases via `resource instanceof HTMLCanvasElement` (e.g. PixiJS
    // CanvasSource.test → otherwise "Could not find a source type for resource").
    // Duck-type so a native migo canvas passes, while still accepting any real
    // prototype-chain instance (Symbol.hasInstance replaces the default check).
    static [Symbol.hasInstance](obj) {
      if (obj == null) return false;
      if (typeof obj.getContext === "function" && typeof obj.width === "number" && typeof obj.height === "number") {
        return true;
      }
      for (let p = Object.getPrototypeOf(obj); p; p = Object.getPrototypeOf(p)) {
        if (p === _HTMLCanvasElement.prototype) return true;
      }
      return false;
    }
  };
  var HTMLAudioElement = class extends HTMLElement {
    constructor() {
      super("audio");
    }
  };
  var HTMLMediaElement = class extends HTMLElement {
    constructor(tag = "media") {
      super(tag);
    }
  };
  var HTMLVideoElement = class extends HTMLElement {
    constructor() {
      super("video");
    }
  };

  // src/image.js
  function Image() {
    if (typeof migo.createImage !== "function") {
      throw new Error("[migo-adapter] migo.createImage is not available");
    }
    return migo.createImage();
  }

  // src/canvas.js
  function Canvas() {
    if (typeof migo.createCanvas !== "function") {
      throw new Error("[migo-adapter] migo.createCanvas is not available");
    }
    const c = migo.createCanvas();
    if (typeof c.addEventListener !== "function") {
      c.addEventListener = () => {
      };
      c.removeEventListener = () => {
      };
      c.dispatchEvent = () => {
      };
    }
    return c;
  }

  // src/audio.js
  var _audioFinalizer = typeof FinalizationRegistry === "function" ? new FinalizationRegistry((ctx) => {
    try {
      if (ctx && typeof ctx.destroy === "function") ctx.destroy();
    } catch (_) {
    }
  }) : null;
  var Audio = class _Audio extends EventTarget {
    constructor(src) {
      super();
      if (typeof migo.createInnerAudioContext !== "function") {
        throw new Error("[migo-adapter] migo.createInnerAudioContext is not available");
      }
      this._ctx = migo.createInnerAudioContext();
      this._readyState = 0;
      if (typeof WeakRef === "function") {
        const selfRef = new WeakRef(this);
        const dispatchSelf = (type) => () => {
          const self = selfRef.deref();
          if (!self) return;
          if (type === "canplay") self._readyState = 4;
          self.dispatchEvent({ type });
        };
        this._ctx.onCanplay && this._ctx.onCanplay(dispatchSelf("canplay"));
        this._ctx.onPlay && this._ctx.onPlay(dispatchSelf("play"));
        this._ctx.onPause && this._ctx.onPause(dispatchSelf("pause"));
        this._ctx.onEnded && this._ctx.onEnded(dispatchSelf("ended"));
        this._ctx.onError && this._ctx.onError((err) => {
          const self = selfRef.deref();
          if (self) self.dispatchEvent({ type: "error", error: err });
        });
      } else {
        const dispatchSelf = (type) => () => {
          if (type === "canplay") this._readyState = 4;
          this.dispatchEvent({ type });
        };
        this._ctx.onCanplay && this._ctx.onCanplay(dispatchSelf("canplay"));
        this._ctx.onPlay && this._ctx.onPlay(dispatchSelf("play"));
        this._ctx.onPause && this._ctx.onPause(dispatchSelf("pause"));
        this._ctx.onEnded && this._ctx.onEnded(dispatchSelf("ended"));
        this._ctx.onError && this._ctx.onError((err) => this.dispatchEvent({ type: "error", error: err }));
      }
      if (_audioFinalizer) _audioFinalizer.register(this, this._ctx, this);
      if (src) this.src = src;
    }
    set src(v) {
      this._ctx.src = v;
    }
    get src() {
      return this._ctx.src;
    }
    set loop(v) {
      this._ctx.loop = v;
    }
    get loop() {
      return this._ctx.loop;
    }
    set volume(v) {
      this._ctx.volume = v;
    }
    get volume() {
      return this._ctx.volume;
    }
    set autoplay(v) {
      this._ctx.autoplay = v;
    }
    get autoplay() {
      return this._ctx.autoplay;
    }
    get currentTime() {
      return this._ctx.currentTime || 0;
    }
    set currentTime(v) {
      if (typeof this._ctx.seek === "function") this._ctx.seek(v);
    }
    get duration() {
      return this._ctx.duration || 0;
    }
    get paused() {
      return !!this._ctx.paused;
    }
    get readyState() {
      return this._readyState;
    }
    play() {
      this._ctx.play();
    }
    pause() {
      this._ctx.pause();
    }
    load() {
    }
    // no-op — InnerAudioContext loads on src set / play
    cloneNode() {
      return new _Audio(this.src);
    }
    // Not a standard HTMLMediaElement method, but exposed so callers can release
    // the native context deterministically instead of waiting for GC.
    destroy() {
      if (_audioFinalizer) _audioFinalizer.unregister(this);
      if (this._ctx && typeof this._ctx.destroy === "function") this._ctx.destroy();
    }
  };

  // src/document.js
  var _docTarget = new EventTarget();
  var document = {
    // Starts "loading"; index.js walks it "loading" → "interactive" (fires
    // DOMContentLoaded) → "complete" (fires window `load`) on a deferred
    // macrotask, mirroring how a browser drives a `<script defer>` page. Browser
    // engines (Phaser, Egret, …) boot from those events, so they must fire.
    readyState: "loading",
    onreadystatechange: null,
    visibilityState: "visible",
    hidden: false,
    documentElement: null,
    // patched at the end (circular: window → document)
    location: location_default,
    ontouchstart: null,
    ontouchmove: null,
    ontouchend: null,
    ontouchcancel: null,
    style: {},
    head: new HTMLElement("head"),
    body: new HTMLElement("body"),
    // Set true by index.js when the `load` event fires; gates display-canvas
    // routing below so it only applies to canvases created during engine boot.
    _loadFired: false,
    _mainCanvasRouted: false,
    createElement(tag) {
      const t = String(tag).toLowerCase();
      if (t === "canvas") {
        if (this._loadFired && !this._mainCanvasRouted && globalThis.canvas && !globalThis.canvas._context) {
          this._mainCanvasRouted = true;
          return globalThis.canvas;
        }
        return new Canvas();
      }
      if (t === "img" || t === "image") return new Image();
      if (t === "audio") return new Audio();
      return new HTMLElement(tag);
    },
    createElementNS(_ns, tag) {
      return this.createElement(tag);
    },
    createTextNode(text) {
      return { nodeType: 3, textContent: String(text), nodeValue: String(text) };
    },
    getElementById(id) {
      if (globalThis.canvas && (globalThis.canvas.id === id || id === "GameCanvas")) {
        return globalThis.canvas;
      }
      return null;
    },
    getElementsByTagName(_tag) {
      return [];
    },
    getElementsByName(_name) {
      return [];
    },
    getElementsByClassName(_cls) {
      return [];
    },
    querySelector(_q) {
      return null;
    },
    querySelectorAll(_q) {
      return [];
    },
    addEventListener(type, listener) {
      _docTarget.addEventListener(type, listener);
    },
    removeEventListener(type, listener) {
      _docTarget.removeEventListener(type, listener);
    },
    dispatchEvent(event) {
      return _docTarget.dispatchEvent(event);
    }
  };
  var document_default = document;

  // src/events.js
  var Event = class {
    constructor(type, init = {}) {
      this.type = type;
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.target = null;
      this.currentTarget = null;
      this.timeStamp = Date.now();
      this.defaultPrevented = false;
    }
    preventDefault() {
      if (this.cancelable) this.defaultPrevented = true;
    }
    stopPropagation() {
    }
    stopImmediatePropagation() {
    }
  };
  var TouchEvent = class extends Event {
    constructor(type, init = {}) {
      super(type, init);
      this.touches = init.touches || [];
      this.targetTouches = init.targetTouches || this.touches;
      this.changedTouches = init.changedTouches || this.touches;
    }
  };
  var MouseEvent = class extends Event {
    constructor(type, init = {}) {
      super(type, init);
      this.clientX = init.clientX || 0;
      this.clientY = init.clientY || 0;
      this.pageX = init.pageX || this.clientX;
      this.pageY = init.pageY || this.clientY;
      this.button = init.button || 0;
      this.buttons = init.buttons || 0;
    }
  };
  var DeviceMotionEvent = class extends Event {
    constructor(type, init = {}) {
      super(type, init);
      this.acceleration = init.acceleration || null;
      this.accelerationIncludingGravity = init.accelerationIncludingGravity || null;
      this.rotationRate = init.rotationRate || null;
      this.interval = init.interval || 0;
    }
  };

  // src/local-storage.js
  var localStorage = {
    get length() {
      try {
        return migo.getStorageInfoSync().keys.length;
      } catch {
        return 0;
      }
    },
    key(i) {
      try {
        return migo.getStorageInfoSync().keys[i] || null;
      } catch {
        return null;
      }
    },
    getItem(k) {
      try {
        const v = migo.getStorageSync(k);
        return v === "" ? null : v;
      } catch {
        return null;
      }
    },
    setItem(k, v) {
      try {
        migo.setStorageSync(k, String(v));
      } catch {
      }
    },
    removeItem(k) {
      try {
        migo.removeStorageSync(k);
      } catch {
      }
    },
    clear() {
      try {
        migo.clearStorageSync();
      } catch {
      }
    }
  };
  var local_storage_default = localStorage;

  // src/xhr.js
  var UNSENT = 0;
  var OPENED = 1;
  var HEADERS_RECEIVED = 2;
  var LOADING = 3;
  var DONE = 4;
  var XMLHttpRequest = class extends EventTarget {
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
      this.responseType = "";
      this.timeout = 0;
      this.withCredentials = false;
      this.onreadystatechange = null;
      this.onload = null;
      this.onerror = null;
      this.ontimeout = null;
      this.onabort = null;
    }
    open(method, url) {
      this._method = String(method).toUpperCase();
      this._url = url;
      this._setReady(OPENED);
    }
    setRequestHeader(name, value) {
      this._headers[name] = value;
    }
    getResponseHeader() {
      return null;
    }
    getAllResponseHeaders() {
      return "";
    }
    abort() {
      if (this._task && typeof this._task.abort === "function") this._task.abort();
      if (this.onabort) try {
        this.onabort({ type: "abort" });
      } catch {
      }
    }
    send(body) {
      if (typeof migo.request !== "function") {
        throw new Error("[migo-adapter] migo.request is not available");
      }
      const dataType = this.responseType === "json" ? "json" : void 0;
      const responseType = this.responseType === "arraybuffer" ? "arraybuffer" : "text";
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
          if (this.onload) try {
            this.onload({ type: "load", target: this });
          } catch {
          }
        },
        fail: (err) => {
          this.status = 0;
          this._setReady(DONE);
          if (this.onerror) try {
            this.onerror({ type: "error", error: err });
          } catch {
          }
        }
      });
    }
    _setReady(s) {
      this.readyState = s;
      if (this.onreadystatechange) try {
        this.onreadystatechange({ type: "readystatechange" });
      } catch {
      }
    }
  };
  XMLHttpRequest.UNSENT = UNSENT;
  XMLHttpRequest.OPENED = OPENED;
  XMLHttpRequest.HEADERS_RECEIVED = HEADERS_RECEIVED;
  XMLHttpRequest.LOADING = LOADING;
  XMLHttpRequest.DONE = DONE;

  // src/websocket.js
  var CONNECTING = 0;
  var OPEN = 1;
  var CLOSING = 2;
  var CLOSED = 3;
  var WebSocket = class extends EventTarget {
    constructor(url, protocols) {
      super();
      if (typeof migo.connectSocket !== "function") {
        throw new Error("[migo-adapter] migo.connectSocket is not available");
      }
      this.url = url;
      this.protocol = Array.isArray(protocols) ? protocols.join(",") : protocols || "";
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
        if (this.onopen) try {
          this.onopen(ev);
        } catch {
        }
        this.dispatchEvent(ev);
      });
      this._task.onMessage && this._task.onMessage((res) => {
        const ev = { type: "message", data: res.data, target: this };
        if (this.onmessage) try {
          this.onmessage(ev);
        } catch {
        }
        this.dispatchEvent(ev);
      });
      this._task.onClose && this._task.onClose((res) => {
        this.readyState = CLOSED;
        const ev = { type: "close", code: res && res.code, reason: res && res.reason, target: this };
        if (this.onclose) try {
          this.onclose(ev);
        } catch {
        }
        this.dispatchEvent(ev);
      });
      this._task.onError && this._task.onError((err) => {
        const ev = { type: "error", error: err, target: this };
        if (this.onerror) try {
          this.onerror(ev);
        } catch {
        }
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
  };
  WebSocket.CONNECTING = CONNECTING;
  WebSocket.OPEN = OPEN;
  WebSocket.CLOSING = CLOSING;
  WebSocket.CLOSED = CLOSED;

  // src/file-reader.js
  var EMPTY = 0;
  var LOADING2 = 1;
  var DONE2 = 2;
  var FileReader = class extends EventTarget {
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
      this.readyState = DONE2;
      if (this.onabort) try {
        this.onabort({ type: "abort" });
      } catch {
      }
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
      this.readyState = LOADING2;
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
            this.result = "data:application/octet-stream;base64,";
          }
          this.readyState = DONE2;
          if (this.onload) try {
            this.onload({ type: "load", target: this });
          } catch {
          }
          if (this.onloadend) try {
            this.onloadend({ type: "loadend", target: this });
          } catch {
          }
        } catch (e) {
          this.readyState = DONE2;
          this.error = e;
          if (this.onerror) try {
            this.onerror({ type: "error", error: e });
          } catch {
          }
        }
      });
    }
  };
  FileReader.EMPTY = EMPTY;
  FileReader.LOADING = LOADING2;
  FileReader.DONE = DONE2;

  // src/intl.js
  function segment(str) {
    str = String(str);
    const parts = Array.from(str);
    return {
      [Symbol.iterator]() {
        let i = 0;
        return {
          next() {
            if (i < parts.length) {
              const value = { segment: parts[i], index: i, input: str, isWordLike: true };
              i++;
              return { value, done: false };
            }
            return { value: void 0, done: true };
          }
        };
      },
      containing(index = 0) {
        const i = Math.max(0, Math.min(parts.length - 1, index | 0));
        return { segment: parts[i] || "", index: i, input: str };
      }
    };
  }
  var Intl = {
    Segmenter: class {
      segment(input) {
        return segment(input);
      }
      resolvedOptions() {
        return {};
      }
    },
    NumberFormat: class {
      format(n) {
        return String(n);
      }
      formatToParts(n) {
        return [{ type: "integer", value: String(n) }];
      }
      resolvedOptions() {
        return {};
      }
    },
    DateTimeFormat: class {
      format(d) {
        return String(d);
      }
      formatToParts() {
        return [];
      }
      resolvedOptions() {
        return { timeZone: "UTC" };
      }
    },
    Collator: class {
      compare(a, b) {
        return a < b ? -1 : a > b ? 1 : 0;
      }
      resolvedOptions() {
        return {};
      }
    },
    PluralRules: class {
      select() {
        return "other";
      }
      resolvedOptions() {
        return {};
      }
    },
    getCanonicalLocales(locales) {
      return [].concat(locales || []);
    }
  };
  var intl_default = Intl;

  // src/index.js
  if (!globalThis.__migoAdapterInjected) {
    globalThis.__migoAdapterInjected = true;
    const canvas = new Canvas();
    canvas.id = "GameCanvas";
    globalThis.canvas = canvas;
    const _winTarget = new EventTarget();
    const _forward = (type) => (e) => {
      const ev = { ...e, type, target: canvas };
      canvas.dispatchEvent && canvas.dispatchEvent(ev);
      document_default.dispatchEvent(ev);
      _winTarget.dispatchEvent(ev);
      const sink = document_default["on" + type];
      if (typeof sink === "function") try {
        sink(ev);
      } catch {
      }
      const wsink = globalThis["on" + type];
      if (typeof wsink === "function") try {
        wsink(ev);
      } catch {
      }
    };
    if (typeof migo.onTouchStart === "function") migo.onTouchStart(_forward("touchstart"));
    if (typeof migo.onTouchMove === "function") migo.onTouchMove(_forward("touchmove"));
    if (typeof migo.onTouchEnd === "function") migo.onTouchEnd(_forward("touchend"));
    if (typeof migo.onTouchCancel === "function") migo.onTouchCancel(_forward("touchcancel"));
    document_default.documentElement = globalThis;
    const surface = {
      // BOM scalars (data values, snapshotted; bom.js refreshes on resize)
      innerWidth,
      innerHeight,
      outerWidth,
      outerHeight,
      screenWidth,
      screenHeight,
      devicePixelRatio,
      screen,
      // BOM stubs
      navigator: navigator_default,
      location: location_default,
      // DOM
      document: document_default,
      // Constructors / classes
      HTMLElement,
      Element,
      Node,
      HTMLImageElement,
      HTMLCanvasElement,
      HTMLAudioElement,
      HTMLMediaElement,
      HTMLVideoElement,
      EventTarget,
      Event,
      TouchEvent,
      MouseEvent,
      DeviceMotionEvent,
      Image,
      Audio,
      XMLHttpRequest,
      WebSocket,
      FileReader,
      localStorage: local_storage_default,
      // window EventTarget surface (touch/resize/etc.). Games commonly do
      // `window.addEventListener('touchstart', ...)`.
      addEventListener: (type, listener, opts) => _winTarget.addEventListener(type, listener, opts),
      removeEventListener: (type, listener, opts) => _winTarget.removeEventListener(type, listener, opts),
      dispatchEvent: (event) => _winTarget.dispatchEvent(event)
    };
    if (!globalThis.Intl) surface.Intl = intl_default;
    for (const key of Object.keys(surface)) {
      try {
        Object.defineProperty(globalThis, key, {
          value: surface[key],
          writable: true,
          enumerable: true,
          configurable: true
        });
      } catch {
      }
    }
    if (!globalThis.window) globalThis.window = globalThis;
    if (!globalThis.self) globalThis.self = globalThis;
    if (!globalThis.parent) globalThis.parent = globalThis;
    if (!globalThis.top) globalThis.top = globalThis;
    if (typeof globalThis.addEventListener !== "function") {
      globalThis.addEventListener = (t, l) => document_default.addEventListener(t, l);
      globalThis.removeEventListener = (t, l) => document_default.removeEventListener(t, l);
      globalThis.dispatchEvent = (e) => document_default.dispatchEvent(e);
    }
    const _setReadyState = (state) => {
      document_default.readyState = state;
      const ev = { type: "readystatechange", target: document_default, currentTarget: document_default };
      document_default.dispatchEvent(ev);
      if (typeof document_default.onreadystatechange === "function") {
        try {
          document_default.onreadystatechange(ev);
        } catch {
        }
      }
    };
    const _fireDomLifecycle = () => {
      _setReadyState("interactive");
      const domReady = { type: "DOMContentLoaded", target: document_default, currentTarget: document_default };
      document_default.dispatchEvent(domReady);
      _winTarget.dispatchEvent(domReady);
      _setReadyState("complete");
      document_default._loadFired = true;
      const load = { type: "load", target: globalThis, currentTarget: globalThis };
      _winTarget.dispatchEvent(load);
      document_default.dispatchEvent(load);
      if (typeof globalThis.onload === "function") {
        try {
          globalThis.onload(load);
        } catch {
        }
      }
      if (typeof document_default.onload === "function") {
        try {
          document_default.onload(load);
        } catch {
        }
      }
    };
    if (typeof setTimeout === "function") setTimeout(_fireDomLifecycle, 0);
    else if (typeof queueMicrotask === "function") queueMicrotask(_fireDomLifecycle);
    else Promise.resolve().then(_fireDomLifecycle);
  }
  var index_default = globalThis;
})();
