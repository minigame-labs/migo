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
  readyState: "complete",
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

  createElement(tag) {
    const t = String(tag).toLowerCase();
    if (t === "canvas") return new Canvas();
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
