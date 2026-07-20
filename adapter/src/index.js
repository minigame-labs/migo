// migo-adapter entry point. Layers a browser-style BOM/DOM surface on top of
// the migo runtime so that engines built for browser-like environments
// (Cocos Creator, Egret, Laya, Pixi, raw WebGL) can run unchanged.
//
// Usage (game side, before the engine boots):
//
//   import "@minigame-labs/migo-adapter";          // ESM
//   // or, in CommonJS / require-style:
//   require("./adapter/src/index.js");
//
// One-shot: idempotent on re-entry.

import * as bom from "./bom.js";
import navigator from "./navigator.js";
import location from "./location.js";
import document from "./document.js";
import HTMLElement, { Node, Element, HTMLImageElement, HTMLCanvasElement,
                       HTMLAudioElement, HTMLMediaElement, HTMLVideoElement } from "./element.js";
import EventTarget from "./event-target.js";
import { Event, TouchEvent, MouseEvent, DeviceMotionEvent } from "./events.js";
import { GamepadEvent, connectGamepadEvents } from "./gamepad.js";
import Image from "./image.js";
import Canvas from "./canvas.js";
import Audio from "./audio.js";
import localStorage from "./local-storage.js";
import XMLHttpRequest from "./xhr.js";
import WebSocket from "./websocket.js";
import FileReader from "./file-reader.js";
import Intl from "./intl.js";

if (!globalThis.__migoAdapterInjected) {
  globalThis.__migoAdapterInjected = true;

  // 1. Global on-screen canvas. Engines do `document.getElementById('GameCanvas')`
  //    or grab `window.canvas`. Create once, reuse.
  const canvas = new Canvas();
  canvas.id = "GameCanvas";
  globalThis.canvas = canvas;

  // 2. Forward host touch events to the canvas + document + window.
  //    Browsers fire touch events on the target element AND they bubble to
  //    document AND window, so games listen on any of the three. Migo's
  //    globalThis has no native EventTarget, so back `window` with our own.
  const _winTarget = new EventTarget();
  const _forward = (type) => (e) => {
    // `type` must come AFTER `...e`: the host event may carry its own `type`
    // field which would otherwise clobber the DOM event name and make
    // `addEventListener('touchstart', ...)` never match.
    const ev = { ...e, type, target: canvas };
    canvas.dispatchEvent && canvas.dispatchEvent(ev);
    document.dispatchEvent(ev);
    _winTarget.dispatchEvent(ev);
    // Also trigger document.ontouch* / window.ontouch* sinks if engines set
    // those directly.
    const sink = document["on" + type];
    if (typeof sink === "function") try { sink(ev); } catch {}
    const wsink = globalThis["on" + type];
    if (typeof wsink === "function") try { wsink(ev); } catch {}
  };
  if (typeof migo.onTouchStart === "function") migo.onTouchStart(_forward("touchstart"));
  if (typeof migo.onTouchMove === "function") migo.onTouchMove(_forward("touchmove"));
  if (typeof migo.onTouchEnd === "function") migo.onTouchEnd(_forward("touchend"));
  if (typeof migo.onTouchCancel === "function") migo.onTouchCancel(_forward("touchcancel"));

  // 2b. Gamepad connection events. Browsers fire these on window only -- not on
  //     document and not on the canvas -- so unlike touch above this routes to
  //     exactly one target.
  connectGamepadEvents((event) => {
    _winTarget.dispatchEvent(event);
    const sink = globalThis["on" + event.type];
    if (typeof sink === "function") try { sink(event); } catch {}
  });

  // 3. Patch the document → window self-reference.
  document.documentElement = globalThis;

  // 4. Publish the BOM/DOM surface on globalThis. Properties go through
  //    Object.defineProperty so that subsequent assignment by engine code
  //    (e.g. `window.innerWidth = ...`) works (writable: true).
  const surface = {
    // BOM scalars (data values, snapshotted; bom.js refreshes on resize)
    innerWidth: bom.innerWidth, innerHeight: bom.innerHeight,
    outerWidth: bom.outerWidth, outerHeight: bom.outerHeight,
    screenWidth: bom.screenWidth, screenHeight: bom.screenHeight,
    devicePixelRatio: bom.devicePixelRatio,
    screen: bom.screen,
    // BOM stubs
    navigator, location,
    // DOM
    document,
    // Constructors / classes
    HTMLElement, Element, Node,
    HTMLImageElement, HTMLCanvasElement, HTMLAudioElement,
    HTMLMediaElement, HTMLVideoElement,
    EventTarget, Event, TouchEvent, MouseEvent, DeviceMotionEvent, GamepadEvent,
    Image, Audio,
    XMLHttpRequest, WebSocket, FileReader,
    localStorage,
    // window EventTarget surface (touch/resize/etc.). Games commonly do
    // `window.addEventListener('touchstart', ...)`.
    addEventListener: (type, listener, opts) => _winTarget.addEventListener(type, listener, opts),
    removeEventListener: (type, listener, opts) => _winTarget.removeEventListener(type, listener, opts),
    dispatchEvent: (event) => _winTarget.dispatchEvent(event),
  };

  // `Intl` is absent under Migo's no-i18n V8; provide the polyfill, but never
  // clobber a real `Intl` (e.g. a future ICU-enabled build or a host browser).
  if (!globalThis.Intl) surface.Intl = Intl;

  for (const key of Object.keys(surface)) {
    try {
      Object.defineProperty(globalThis, key, {
        value: surface[key],
        writable: true,
        enumerable: true,
        configurable: true,
      });
    } catch {
      // some host-defined globals may be non-configurable; skip them.
    }
  }

  // 5. Window self-references that engines depend on.
  if (!globalThis.window) globalThis.window = globalThis;
  if (!globalThis.self) globalThis.self = globalThis;
  if (!globalThis.parent) globalThis.parent = globalThis;
  if (!globalThis.top) globalThis.top = globalThis;

  // 6. addEventListener on the window: route to document.
  if (typeof globalThis.addEventListener !== "function") {
    globalThis.addEventListener = (t, l) => document.addEventListener(t, l);
    globalThis.removeEventListener = (t, l) => document.removeEventListener(t, l);
    globalThis.dispatchEvent = (e) => document.dispatchEvent(e);
  }

  // 7. DOM lifecycle events. Browser-targeted engines commonly boot from
  //    `window.addEventListener('load', ...)` or `DOMContentLoaded` (e.g.
  //    Phaser's game entry is `window.addEventListener('load', () => new
  //    Phaser.Game(cfg))`). A real browser fires these AFTER a `<script defer>`
  //    game has run and registered its listeners. The adapter+game prelude runs
  //    in one synchronous turn, so defer to a macrotask — by then the game's
  //    top-level code has registered its listeners. readyState walks
  //    "loading" → "interactive" (DOMContentLoaded) → "complete" (load),
  //    matching the sequence a deferred script observes on the web. Fires once
  //    (the whole block is guarded by `__migoAdapterInjected`).
  const _setReadyState = (state) => {
    document.readyState = state;
    const ev = { type: "readystatechange", target: document, currentTarget: document };
    document.dispatchEvent(ev);
    if (typeof document.onreadystatechange === "function") {
      try { document.onreadystatechange(ev); } catch {}
    }
  };
  const _fireDomLifecycle = () => {
    _setReadyState("interactive");
    const domReady = { type: "DOMContentLoaded", target: document, currentTarget: document };
    document.dispatchEvent(domReady);
    _winTarget.dispatchEvent(domReady); // some libs listen for it on window
    _setReadyState("complete");
    // Gate display-canvas routing (document.js) to boot-time canvases: the
    // load listeners below are where engines construct and create their canvas.
    document._loadFired = true;
    const load = { type: "load", target: globalThis, currentTarget: globalThis };
    _winTarget.dispatchEvent(load);     // window 'load' listeners
    document.dispatchEvent(load);        // and document, for engines that listen there
    if (typeof globalThis.onload === "function") { try { globalThis.onload(load); } catch {} }
    if (typeof document.onload === "function") { try { document.onload(load); } catch {} }
  };
  // Defer past the current synchronous turn. Prefer a macrotask (matches the
  // browser, where load is a task, not a microtask); fall back if unavailable.
  if (typeof setTimeout === "function") setTimeout(_fireDomLifecycle, 0);
  else if (typeof queueMicrotask === "function") queueMicrotask(_fireDomLifecycle);
  else Promise.resolve().then(_fireDomLifecycle);
}

export default globalThis;
