# migo-wx-adapter

Publishes `globalThis.wx`, aliased to the [migo](../) mini-game runtime's own `migo.*` capabilities. Lets mini-game content written against the `wx` global -- unmodified WeChat mini-game source, or content from a similarly-shaped mini-game platform -- run on migo unchanged.

The migo runtime installs only `migo` by default. `wx` is not built in, at any scale -- this adapter is how a game or host opts into it.

## When to use

- Your content is real, unmodified wx mini-game source (`wx.createCanvas()`, `wx.getSystemInfoSync()`, `wx.onTouchStart()`, ...).
- You're porting a WeChat mini-game and want to verify it runs before touching its source.

If your content calls `migo.*` directly, you don't need this adapter.

## What this is NOT

A reimplementation of wx. For every capability migo also implements, `wx.foo` and `migo.foo` are **the same function reference** -- this adapter is a naming/shaping layer over migo's own capabilities, not a new implementation of them. If migo doesn't implement a given wx API, `wx.thatApi` is `undefined` here exactly as it is on `migo` directly; no adapter can manufacture a capability the runtime doesn't have.

Concretely, that makes this adapter genuinely small: the whole thing is copying migo's own property descriptors onto a new object, minus a short, explicit exclusion list (see below). There's no protocol translation, no polyfilling, no behavioral shimming -- that's the difference between this and the [BOM/DOM adapter](../adapter/), which does real work because browsers and wx mini-games don't share a capability model. wx mini-games and migo do.

## Install

Plain ESM source -- no build step required.

```js
// game entry, BEFORE content that references `wx` runs
import "@minigame-labs/migo-wx-adapter";
// or, with a require/AMD loader:
require("./wx-adapter/src/index.js");
```

The adapter detects re-entry via `globalThis.__migoWxAdapterInjected` and is safe to import twice. It's also safe to load alongside `@minigame-labs/migo-adapter` (BOM/DOM) -- they touch disjoint globals.

### Zero-touch testing via runtime boot prelude

Same mechanism as the BOM/DOM adapter: build the IIFE bundle once and feed it to the runtime via `InitOptions::with_prelude_script`, so third-party wx content that doesn't import this adapter itself still gets `wx` wired up before it runs.

```sh
cd wx-adapter
npm run build
# → wx-adapter/dist/migo-wx-adapter.bundle.js
```

```rust
let bundle = std::fs::read_to_string("path/to/migo-wx-adapter.bundle.js")?;
let init = InitOptions::new()
    .with_prelude_script("<migo-wx-adapter>", bundle)
    // ... other options
    ;
```

On Android, via `RuntimeConfig.Builder`:

```java
String bundle = readAssetAsString(context, "migo-wx-adapter.bundle.js");
RuntimeConfig config = new RuntimeConfig.Builder(context)
        .addPreludeScript("<migo-wx-adapter>", bundle)
        .build();
```

## What's excluded from `wx`

Capabilities migo exposes beyond the common mini-game surface -- present for browser-content interop (the Web Gamepad API), but with no wx equivalent on real WeChat:

| Name | Reason |
|---|---|
| `getGamepads`, `onGamepadConnected`, `offGamepadConnected`, `onGamepadDisconnected`, `offGamepadDisconnected` | Browser content capability, not a wx API. Left off `wx` so feature-detection ported from real wx content (`if (!wx.getGamepads) { ... }`) sees the same absence it would on WeChat. Still reachable on `migo` directly. |

This list mirrors `_NON_MINIGAME_API` in the engine's own `97_migo_namespace.js`; if that set ever changes, update both together.

## Layout

```
src/
  index.js          entry -- builds and publishes globalThis.wx
scripts/
  build-bundle.mjs   esbuild → dist/migo-wx-adapter.bundle.js (IIFE; for prelude injection)
tests/
  adapter.test.mjs   ESM behavior against a fake migo runtime
  bundle.test.mjs    IIFE bundle smoke-tested in a vm.Context, including the
                     idempotency guard and the "migo not booted" failure mode
```

## Running tests

```sh
cd wx-adapter
node tests/adapter.test.mjs
node tests/bundle.test.mjs
# or
npm test                 # runs both
npm run build && npm test  # rebuild bundle then test
```

## License

MIT
