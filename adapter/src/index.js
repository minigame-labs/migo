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
import { Event, TouchEvent, MouseEvent, WheelEvent, DeviceMotionEvent } from "./events.js";
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

  // Dispatch a real (cancelable) Event to canvas + document + window + the
  // `on<type>` sinks engines sometimes set directly. Returns whether a listener
  // called preventDefault() -- real Event objects are required so that call
  // works at all (the previous plain-object spread had no preventDefault, so a
  // touchstart handler calling it threw and was swallowed).
  const _emit = (ev) => {
    ev.target = canvas;
    canvas.dispatchEvent && canvas.dispatchEvent(ev);
    document.dispatchEvent(ev);
    _winTarget.dispatchEvent(ev);
    const sink = document["on" + ev.type];
    if (typeof sink === "function") try { sink(ev); } catch {}
    const wsink = globalThis["on" + ev.type];
    if (typeof wsink === "function") try { wsink(ev); } catch {}
    return ev.defaultPrevented;
  };

  // ---- Touch -> DOM touch events, tracking W3C compat-mouse suppression. ----
  // `_touchCompat` is null when no touch interaction is concurrent; otherwise it
  // is the current interaction's touchstart `defaultPrevented`, which decides
  // whether the paired compatibility mouse events are dropped. See
  // docs/superpowers/specs/2026-07-25-desktop-pointer-touch-compat-events-design.md.
  let _touchCompat = null;
  const _forwardTouch = (type) => (e) => {
    const ev = new TouchEvent(type, {
      bubbles: true,
      cancelable: true,
      touches: e.touches || [],
      changedTouches: e.changedTouches || e.touches || [],
    });
    ev.timeStamp = e.timeStamp;
    const prevented = _emit(ev);
    if (type === "touchstart") {
      _touchCompat = prevented;
    } else if (type === "touchend" || type === "touchcancel") {
      // Keep the flag through the (microtask-deferred) compat-mouse burst, then
      // clear on a macrotask so a later standalone mouse click is not gated.
      setTimeout(() => { _touchCompat = null; }, 0);
    }
  };
  if (typeof migo.onTouchStart === "function") migo.onTouchStart(_forwardTouch("touchstart"));
  if (typeof migo.onTouchMove === "function") migo.onTouchMove(_forwardTouch("touchmove"));
  if (typeof migo.onTouchEnd === "function") migo.onTouchEnd(_forwardTouch("touchend"));
  if (typeof migo.onTouchCancel === "function") migo.onTouchCancel(_forwardTouch("touchcancel"));

  // ---- Mouse -> DOM as W3C compatibility mouse events. ----------------------
  // The embedded layer delivers mouse synchronously but touch on a microtask,
  // so the adapter sees mouse before touch regardless of host send order.
  // Deferring the mouse dispatch by TWO microtasks guarantees it runs after the
  // single-microtask touch drain, so the touchstart's preventDefault is known
  // before we decide whether this compat mouse event is suppressed.
  const _emitCompatMouse = (type, src, extra) => {
    Promise.resolve().then().then(() => {
      if (_touchCompat === true) return; // paired touch was preventDefault()ed
      const ev = new MouseEvent(type, {
        bubbles: true,
        cancelable: true,
        clientX: src.x,
        clientY: src.y,
        button: src.button,
        buttons: type === "mouseup" ? 0 : 1,
        ...extra,
      });
      ev.timeStamp = src.timeStamp;
      _emit(ev);
      if (type === "mouseup") {
        // DOM fires `click` after `mouseup`; it is part of the same suppressible
        // compat burst, so it only reaches content when the mouse was not dropped.
        const click = new MouseEvent("click", {
          bubbles: true, cancelable: true, clientX: src.x, clientY: src.y, button: src.button,
        });
        click.timeStamp = src.timeStamp;
        _emit(click);
      }
    });
  };
  if (typeof migo.onMouseDown === "function") migo.onMouseDown((e) => _emitCompatMouse("mousedown", e));
  if (typeof migo.onMouseMove === "function") migo.onMouseMove((e) => _emitCompatMouse("mousemove", e, { movementX: e.movementX, movementY: e.movementY }));
  if (typeof migo.onMouseUp === "function") migo.onMouseUp((e) => _emitCompatMouse("mouseup", e));

  // ---- Wheel -> DOM wheel event (no touch equivalent, no compat gating). ----
  if (typeof migo.onWheel === "function") {
    migo.onWheel((e) => {
      const ev = new WheelEvent("wheel", {
        bubbles: true, cancelable: true,
        deltaX: e.deltaX, deltaY: e.deltaY, deltaZ: e.deltaZ, deltaMode: e.deltaMode,
      });
      ev.timeStamp = e.timeStamp;
      _emit(ev);
    });
  }

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
