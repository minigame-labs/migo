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

  // 2. Forward host touch events to the canvas + document.
  const _forward = (type) => (e) => {
    const ev = { type, ...e, target: canvas };
    canvas.dispatchEvent && canvas.dispatchEvent(ev);
    document.dispatchEvent(ev);
    // Also trigger document.ontouch* sinks if engines set those directly.
    const sink = document["on" + type];
    if (typeof sink === "function") try { sink(ev); } catch {}
  };
  if (typeof migo.onTouchStart === "function") migo.onTouchStart(_forward("touchstart"));
  if (typeof migo.onTouchMove === "function") migo.onTouchMove(_forward("touchmove"));
  if (typeof migo.onTouchEnd === "function") migo.onTouchEnd(_forward("touchend"));
  if (typeof migo.onTouchCancel === "function") migo.onTouchCancel(_forward("touchcancel"));

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
    EventTarget, Event, TouchEvent, MouseEvent, DeviceMotionEvent,
    Image, Audio,
    XMLHttpRequest, WebSocket, FileReader,
    localStorage,
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
}

export default globalThis;
