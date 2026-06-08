# migo-adapter

A browser-style BOM/DOM adapter layered on top of the [migo](../) mini-game runtime. Lets games written for browser-like environments (Cocos Creator, Egret, Laya, Pixi, raw WebGL) run on migo unchanged, by mapping `window.*`, `document.*`, `Image`, `XMLHttpRequest`, etc. onto the corresponding `migo.*` APIs.

The migo runtime intentionally ships **no BOM/DOM** — it offers only `migo.*` (the wx-aligned API surface) plus standard JavaScript. Browser-style globals are this adapter's job.

## When to use

- Your game uses browser globals (`document.createElement`, `new Image()`, `window.innerWidth`, `XMLHttpRequest`, `localStorage`, `WebSocket`, etc.).
- Your engine is built against a browser-like environment (Cocos / Egret / Laya / Pixi / Phaser / Three.js / a custom WebGL stack).
- You want to drop a wx-style game onto migo with minimal changes.

If your game uses only `migo.*` APIs directly (no `window`, no `document`), you don't need this adapter.

## Install

The adapter is plain ESM source — no build step required.

```js
// game entry, BEFORE the engine boots
import "@minigame-labs/migo-adapter";
// or, with a require/AMD loader:
require("./adapter/src/index.js");
```

The adapter detects re-entry via `globalThis.__migoAdapterInjected` and is safe to import twice.

### Zero-touch testing via runtime boot prelude

If you have a third-party browser-style game that does **not** import this
adapter and you don't want to (or can't) modify its source, the migo runtime
can inject the adapter as a *boot prelude script*. Build the IIFE bundle
once and feed it to the runtime via `InitOptions::with_prelude_script`.

Build the bundle:

```sh
cd adapter
npm run build
# → adapter/dist/migo-adapter.bundle.js
```

Wire it into the runtime (Rust side, e.g. desktop launcher or Android JNI
bootstrap):

```rust
let bundle = std::fs::read_to_string("path/to/migo-adapter.bundle.js")?;
let init = InitOptions::new()
    .with_prelude_script("<migo-adapter>", bundle)
    // ... other options
    ;
// then EvaluateModule the game's entry as usual
```

On Android, configure it via `RuntimeConfig.Builder`:

```java
String bundle = readAssetAsString(context, "migo-adapter.bundle.js");
RuntimeConfig config = new RuntimeConfig.Builder(context)
        .addPreludeScript("<migo-adapter>", bundle)
        .build();
```

Drop `migo-adapter.bundle.js` into your app's `assets/` folder so the
host app can read it once at launch and pass the source to the builder.

The prelude runs in the global scope before every `EvaluateModule`, so the
game sees `window.innerWidth`, `document.createElement`, `Image`,
`XMLHttpRequest`, etc. already wired up. Multiple `with_prelude_script`
calls accumulate and execute in declaration order; the same
`__migoAdapterInjected` sentinel makes the bundle safe to combine with a
game that *also* imports the ESM entry.

## What gets exposed on `globalThis`

| Category | Names |
|---|---|
| BOM scalars | `innerWidth`, `innerHeight`, `outerWidth`, `outerHeight`, `screenWidth`, `screenHeight`, `devicePixelRatio` |
| BOM objects | `screen`, `navigator`, `location`, `document`, `localStorage` |
| Window self-references | `window`, `self`, `parent`, `top` |
| Constructors / classes | `Image`, `Audio`, `XMLHttpRequest`, `WebSocket`, `FileReader`, `HTMLElement`, `Element`, `Node`, `EventTarget`, `Event`, `TouchEvent`, `MouseEvent`, `DeviceMotionEvent`, `HTMLImageElement`, `HTMLCanvasElement`, `HTMLAudioElement`, `HTMLMediaElement`, `HTMLVideoElement` |
| On-screen canvas | `globalThis.canvas` (also `document.getElementById("GameCanvas")`) |

## Mapping to `migo.*`

| Browser API | migo runtime call |
|---|---|
| `window.innerWidth` / `screen.width` / `devicePixelRatio` | `migo.getWindowInfo()` (one-shot snapshot at adapter load; assignment persists, host resize does NOT overwrite — see "BOM semantics" below) |
| `navigator.userAgent` / `navigator.platform` | derived from `migo.getSystemInfoSync()` |
| `document.createElement('canvas')` / `new Canvas()` | `migo.createCanvas()` |
| `document.createElement('img')` / `new Image()` | `migo.createImage()` |
| `new Audio(src)` | `migo.createInnerAudioContext()` |
| `localStorage.{getItem, setItem, removeItem, clear}` | `migo.{get, set, remove, clear}StorageSync` |
| `new XMLHttpRequest()` | `migo.request()` |
| `new WebSocket(url)` | `migo.connectSocket()` |
| `addEventListener('touchstart' …)` on `window`, `document`, `canvas` | `migo.onTouchStart` / `onTouchMove` / `onTouchEnd` / `onTouchCancel` |

## BOM semantics

`window.innerWidth`, `innerHeight`, `screen`, `devicePixelRatio` etc. are **one-shot snapshots** of the host window metrics at adapter load time, exposed as plain writable data properties on `globalThis`. This matches the canonical mini-game adapter, and differs from a real browser:

- ✅ `window.innerWidth = X` works and persists.
- ✅ The values are correct for whatever screen the game launched on.
- ❌ The adapter does **not** subscribe to host resize events — if the window resizes (orientation change, foldable expand, IME open), `window.innerWidth` does **not** auto-update.

Games that need live metrics should call `migo.getWindowInfo()` directly (e.g. on a resize listener of their own, or every frame). The adapter intentionally stays out of the way so engine-internal layout caches don't race with adapter writes.

## What this adapter is NOT

- **A full DOM**. We provide a base `Node`/`Element`/`HTMLElement` tree just deep enough that engine bootstrap calls (`appendChild`, `parentNode`, `setAttribute`, `getBoundingClientRect`) don't throw. We do not run a real layout engine, do not match a CSS selector, do not parse HTML. `document.querySelector`, `getElementsByTagName`, etc. return empty results.
- **A full browser networking stack**. `XMLHttpRequest` and `WebSocket` cover the common engine code paths but skip CORS, cookies, response streaming, `Blob`, and `FormData`.
- **A full WebAudio**. `new Audio(src)` is bridged to `migo.createInnerAudioContext`. The `AudioContext` graph (oscillators, filters, etc.) is provided by the migo runtime directly under `globalThis.AudioContext`, not by this adapter.
- **A real `FileReader`**. We support `readAsText`/`readAsArrayBuffer`/`readAsDataURL` against simple inputs (`string`, `ArrayBuffer`). No `Blob` streaming.

If you need any of the above, extend the adapter — the source is small, ESM, and dependency-free.

## Layout

```
src/
  index.js          entry — wires everything onto globalThis
  bom.js            innerWidth / innerHeight / screen / devicePixelRatio
  navigator.js      navigator stub
  location.js       location stub
  document.js       document object (createElement, getElementById, …)
  element.js        Node / Element / HTMLElement and tag-specific subclasses
  event-target.js   EventTarget base class
  events.js         Event / TouchEvent / MouseEvent / DeviceMotionEvent
  image.js          Image() constructor → migo.createImage
  canvas.js         Canvas() constructor → migo.createCanvas
  audio.js          Audio() constructor → migo.createInnerAudioContext
  local-storage.js  localStorage on top of migo.*StorageSync
  xhr.js            XMLHttpRequest on top of migo.request
  websocket.js      WebSocket on top of migo.connectSocket
  file-reader.js    FileReader simple impl
scripts/
  build-bundle.mjs  esbuild → dist/migo-adapter.bundle.js (IIFE; for prelude injection)
tests/
  adapter.test.mjs  Node simulation against a fake migo runtime
  bundle.test.mjs   IIFE bundle smoke-tested in a vm.Context
```

## Running tests

```sh
cd adapter
node tests/adapter.test.mjs
node tests/bundle.test.mjs
# or
npm test                 # runs both
npm run build && npm test  # rebuild bundle then test
```

The ESM test stubs `globalThis.migo` with a fake runtime, imports the adapter,
and asserts BOM/DOM/event/network behavior end-to-end. The bundle test
re-runs a focused subset against `dist/migo-adapter.bundle.js` inside a
`vm.Context` to confirm the IIFE injects the same surface.

## License

MIT
