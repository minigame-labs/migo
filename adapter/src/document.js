// document — minimal surface that engines initialise against.
// createElement(canvas|img|audio) returns a real host object via the wrappers;
// other tags return a plain HTMLElement. getElementById/querySelector return
// null unless asking for the global canvas (engines do `getElementById('GameCanvas')`).

import HTMLElement from "./element.js";
import Image from "./image.js";
import Canvas from "./canvas.js";
import Audio from "./audio.js";
import location from "./location.js";
import EventTarget from "./event-target.js";

const _docTarget = new EventTarget();

const document = {
  // Starts "loading"; index.js walks it "loading" → "interactive" (fires
  // DOMContentLoaded) → "complete" (fires window `load`) on a deferred
  // macrotask, mirroring how a browser drives a `<script defer>` page. Browser
  // engines (Phaser, Egret, …) boot from those events, so they must fire.
  readyState: "loading",
  onreadystatechange: null,
  visibilityState: "visible",
  hidden: false,
  documentElement: null, // patched at the end (circular: window → document)
  location,
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
      // Migo presents only the onscreen canvas (rid 1, exposed as
      // `globalThis.canvas`). Browser engines that create their own render
      // canvas via `document.createElement('canvas')` (e.g. Phaser) would
      // otherwise draw into an offscreen buffer that is never shown. So the
      // first canvas created after `load`, while the onscreen canvas is still
      // unclaimed (no rendering context), is treated as the engine's display
      // canvas and backed by the onscreen surface. Engines that instead reuse
      // the global `canvas` (Pixi/Cocos) claim it before this fires and are
      // unaffected; feature-detection canvases are created before `load`.
      if (this._loadFired && !this._mainCanvasRouted
          && globalThis.canvas && !globalThis.canvas._context) {
        this._mainCanvasRouted = true;
        return globalThis.canvas;
      }
      return new Canvas();
    }
    if (t === "img" || t === "image") return new Image();
    if (t === "audio") return new Audio();
    return new HTMLElement(tag);
  },

  createElementNS(_ns, tag) { return this.createElement(tag); },

  createTextNode(text) { return { nodeType: 3, textContent: String(text), nodeValue: String(text) }; },

  getElementById(id) {
    // Engines often request the on-screen canvas by id. The global canvas is
    // exposed by index.js as both `globalThis.canvas` and bound to a
    // well-known id on first creation.
    if (globalThis.canvas && (globalThis.canvas.id === id || id === "GameCanvas")) {
      return globalThis.canvas;
    }
    return null;
  },

  getElementsByTagName(_tag) { return []; },
  getElementsByName(_name) { return []; },
  getElementsByClassName(_cls) { return []; },
  querySelector(_q) { return null; },
  querySelectorAll(_q) { return []; },

  addEventListener(type, listener) { _docTarget.addEventListener(type, listener); },
  removeEventListener(type, listener) { _docTarget.removeEventListener(type, listener); },
  dispatchEvent(event) { return _docTarget.dispatchEvent(event); },
};

export default document;
