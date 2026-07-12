// Minimal DOM node tree.
// Real games rarely walk the DOM tree, but engines (Cocos/Egret/Pixi) do call
// appendChild / removeChild / parentNode during canvas setup. We track
// parent + children just well enough to make those calls non-throwing.

import EventTarget from "./event-target.js";

// Browser boot pages commonly load the engine/game via a `<script src>` that
// webpack/rollup emit (e.g. Phaser/Egret HTML output: an index.html whose only
// job is `<script src="main.<hash>.js">`). A mini-game host has no HTML parser,
// so nothing loads that chunk and the game never boots. Mini-game runtimes
// (correctly, like WeChat) forbid loading REMOTE scripts, but a script that
// points at a LOCAL file bundled inside the game package is safe: read it via
// the host filesystem and execute it in global scope, then fire the element's
// `load` event so the boot sequence continues. Remote (http/https) srcs are
// refused. This runs once per connected <script src>, deferred to a microtask
// so an `onload` assigned right after `appendChild` is observed.
function maybeLoadScript(node) {
  if (!node || node.tagName !== "SCRIPT" || !node.src || node._migoLoaded) return;
  node._migoLoaded = true;
  Promise.resolve().then(() => {
    const src = String(node.src);
    try {
      if (/^(https?:)?\/\//i.test(src) || /^data:/i.test(src)) {
        console.warn("[migo-adapter] refusing to load non-local script:", src);
        if (typeof node.onerror === "function") node.onerror(new Error("non-local script blocked"));
        node.dispatchEvent && node.dispatchEvent({ type: "error" });
        return;
      }
      const path = src.replace(/^\.?\//, "").split("?")[0].split("#")[0];
      const fs = typeof migo !== "undefined" && migo.getFileSystemManager && migo.getFileSystemManager();
      if (!fs || typeof fs.readFileSync !== "function") {
        throw new Error("no filesystem manager to read local script");
      }
      const code = fs.readFileSync(path, "utf8");
      // Indirect eval → global scope: webpack/engine bundles are top-level
      // IIFEs that install globals; they must not run in a nested lexical scope.
      (0, eval)(code);
      if (typeof node.onload === "function") node.onload();
      node.dispatchEvent && node.dispatchEvent({ type: "load" });
    } catch (e) {
      console.error("[migo-adapter] local script load failed:", src, e);
      if (typeof node.onerror === "function") node.onerror(e);
      node.dispatchEvent && node.dispatchEvent({ type: "error" });
    }
  });
}

export class Node extends EventTarget {
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
    maybeLoadScript(node);
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
    maybeLoadScript(newNode);
    return newNode;
  }

  cloneNode() { return null; }

  // `node.contains(other)` — engines (e.g. PixiJS CanvasSource) call
  // `document.body.contains(canvas)` to decide if a canvas is live in the DOM.
  contains(node) {
    if (node == null) return false;
    if (node === this) return true;
    for (const c of this.children) {
      if (c === node || (typeof c.contains === "function" && c.contains(node))) return true;
    }
    return false;
  }
}

export class Element extends Node {
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
}

export default class HTMLElement extends Element {
  constructor(tagName = "") {
    super();
    this.tagName = String(tagName).toUpperCase();
    this.nodeName = this.tagName;
    this.innerHTML = "";
  }

  get clientWidth() { return globalThis.innerWidth || 0; }
  get clientHeight() { return globalThis.innerHeight || 0; }
  get offsetWidth() { return this.clientWidth; }
  get offsetHeight() { return this.clientHeight; }

  setAttribute(name, value) { this[name] = value; }
  getAttribute(name) { return this[name] == null ? null : this[name]; }
  removeAttribute(name) { delete this[name]; }
  hasAttribute(name) { return name in this; }

  getBoundingClientRect() {
    const w = this.clientWidth, h = this.clientHeight;
    return { top: 0, left: 0, right: w, bottom: h, width: w, height: h, x: 0, y: 0 };
  }

  focus() {}
  blur() {}
  click() {}
}

export class HTMLImageElement extends HTMLElement {
  constructor() { super("img"); }
}

export class HTMLCanvasElement extends HTMLElement {
  constructor() { super("canvas"); }

  // `document.createElement('canvas')` / `new Canvas()` returns the native
  // `migo.createCanvas()` object, which is NOT in this class hierarchy. Engines
  // detect canvases via `resource instanceof HTMLCanvasElement` (e.g. PixiJS
  // CanvasSource.test → otherwise "Could not find a source type for resource").
  // Duck-type so a native migo canvas passes, while still accepting any real
  // prototype-chain instance (Symbol.hasInstance replaces the default check).
  static [Symbol.hasInstance](obj) {
    if (obj == null) return false;
    if (
      typeof obj.getContext === "function" &&
      typeof obj.width === "number" &&
      typeof obj.height === "number"
    ) {
      return true;
    }
    for (let p = Object.getPrototypeOf(obj); p; p = Object.getPrototypeOf(p)) {
      if (p === HTMLCanvasElement.prototype) return true;
    }
    return false;
  }
}

export class HTMLAudioElement extends HTMLElement {
  constructor() { super("audio"); }
}

export class HTMLMediaElement extends HTMLElement {
  constructor(tag = "media") { super(tag); }
}

export class HTMLVideoElement extends HTMLElement {
  constructor() { super("video"); }
}
