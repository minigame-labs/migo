// Minimal DOM node tree.
// Real games rarely walk the DOM tree, but engines (Cocos/Egret/Pixi) do call
// appendChild / removeChild / parentNode during canvas setup. We track
// parent + children just well enough to make those calls non-throwing.

import EventTarget from "./event-target.js";

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
