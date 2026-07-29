# Migo — The Native Runtime for HTML5 & Mini-games

[English](README.md) | [中文](README.zh-CN.md)

[![CI](https://github.com/minigame-labs/migo/actions/workflows/pr-ci.yml/badge.svg)](https://github.com/minigame-labs/migo/actions/workflows/pr-ci.yml)
[![License](https://img.shields.io/badge/license-BSL%201.1-blue.svg)](LICENSE)

**A WebView replacement built for games.** Embed Migo in your app to run HTML5 and mini-game content natively — no browser, no DOM, no CSS, no compositor. Faster startup, lower memory, and a runtime version you pin yourself instead of one that drifts across OEMs and OS updates.

Two adapter profiles let existing games run with zero or minimal changes:

- **Cross-engine HTML5 / Canvas2D / WebGL** — Cocos, Egret, Pixi and vanilla Canvas games run unmodified; the adapter supplies a browser-style BOM/DOM.
- **Mini-game API style** — a mini-game platform–style environment through the `wx` namespace adapter layer.

## Why Migo

| | Migo | Android System WebView |
|---|---|---|
| **Version consistency** | You package and pin the runtime; identical across OEMs and OS versions | Auto-updates with the user's system, outside your control |
| **Auditability** | Source-available; the sandbox boundary is auditable line by line | Closed source |
| **Startup / memory** | No DOM or layout, V8 snapshot warm-up — small footprint, fast start | Ships all of Chromium, heavy resident cost |
| **Cross-engine** | One API across engines | — |

Reproducible benchmarks against the system WebView — same game, same device, same session — live in [migo-bench](https://github.com/minigame-labs/migo-bench).

## Platform support

| Platform | Status | Released artifacts |
|---|---|---|
| **Android** (arm64-v8a, x86_64) | Released | AAR with the Java/Kotlin SDK; a C ABI package per ABI (headers, static library, CMake package) |
| **Linux** (x86_64) | Released | Static and shared library, pkg-config and CMake packages; Qt 6 / X11 host kit in-tree |
| **Windows** (x86_64) | Released | `migo.dll` with its import library, headers, a CMake package, and the ANGLE and V8 runtime DLLs it loads by name |
| iOS, macOS, HarmonyOS | Planned | — |

Released artifacts are on the [releases page](https://github.com/minigame-labs/migo/releases). Each ships an `.attestation.json` recording the archive's name, size and sha256 — verify a download against it before use.

The C ABI in [`include/migo/`](include/migo/) is a **candidate** — it has a working runtime on Android and Linux but is not frozen. Its own README tracks what remains before it can be.

## Quick start

**Integrating Migo into an app** — worked examples for each supported host, with build and run instructions, are in [migo-examples](https://github.com/minigame-labs/migo-examples). Start there rather than from this repository: it carries a runnable game and resolves the runtime artifact for you.

**Building the runtime from source** — see [BUILD.md](BUILD.md) for prerequisites, per-platform setup and the build flow.

```bash
# Android AAR (Linux/macOS host)
./scripts/build-aar.sh release arm64-v8a
```

Prebuilt V8 archives are fetched and verified against their component manifests rather than committed:

```bash
bash scripts/fetch-v8-archives.sh          # Android targets (the build default)
bash scripts/fetch-v8-archives.sh --all    # every target that has a manifest
```

## Architecture

```text
+------------------------------------------------------------------------------------+
|                                      Your App                                      |
+------------------------------------------------------------------------------------+
|                                      Migo SDK                                      |
+---------------------+--------------------+--------------------+--------------------+
|       Graphics      |       Audio        |        I/O         |     JS Runtime     |
|     (Skia / GL)     |     (WebAudio)     |     (File/Net)     |   (deno_core/V8)   |
+---------------------+--------------------+--------------------+--------------------+
|                                  Rust Core Engine                                  |
+------------------------------------------------------------------------------------+
|                     Platform Layer (Android | Linux | Windows)                     |
+------------------------------------------------------------------------------------+
```

## Repository layout

```text
migo/
├── engine/                 # Rust core engine
│   ├── crates/
│   │   ├── core/           # core runtime and session lifecycle
│   │   ├── graphics/       # rendering (Canvas2D, WebGL)
│   │   ├── audio/          # audio
│   │   ├── io/             # file and network I/O
│   │   ├── runtime-v8/     # JavaScript runtime (V8 via deno_core)
│   │   ├── shared/         # shared types and protocol
│   │   ├── platform/       # platform integration
│   │   ├── capi/           # C ABI implementation
│   │   ├── capi-abi/       # C ABI layout and versioning contract
│   │   └── android-jni/    # JNI entry points (libmigo.so)
│   ├── tools/              # snapshot-gen, headless player, C host example
│   └── Cargo.toml
├── adapter/                # HTML5 -> mini-game API adapter (JavaScript)
├── include/migo/           # public C headers
├── platforms/
│   ├── android/            # Android SDK (AAR)
│   ├── linux/              # Linux host kit (Qt 6 / X11)
│   └── windows/            # Windows
├── tests/                  # conformance assets (C ABI lanes, C hosts, probes)
├── contracts/              # artifact manifest schemas
├── scripts/                # build and contract-gate scripts
├── BUILD.md                # building from source
├── LICENSE                 # licence (BSL 1.1)
├── LEGAL.md                # legal notice (licence / trademark / test content)
├── COMMERCIAL.md           # commercial licence: who needs one, who does not
└── NOTICE                  # third-party notices
```

## Related repositories

| Repository | Purpose |
|---|---|
| [migo-examples](https://github.com/minigame-labs/migo-examples) | Host integration examples, one directory per platform |
| [migo-bench](https://github.com/minigame-labs/migo-bench) | Reproducible Migo-vs-WebView benchmarks |
| [migo-test-suite](https://github.com/minigame-labs/migo-test-suite) | Mini-game API conformance test suite |

## License

Migo is **source-available** under the [Business Source License 1.1](LICENSE). **Each released version converts to Apache 2.0 four years after it is published.**

- **Read, audit, build, test, benchmark, modify and port** — granted to everyone, at any scale, unconditionally.
- **Ship Migo inside your own app** — free while under USD 1,000,000 annual revenue and 3,000,000 MAU.
- **Resell Migo as a standalone SDK, or run it as a hosted service** — needs a commercial licence.

See [LEGAL.md](LEGAL.md) for the full statement and [COMMERCIAL.md](COMMERCIAL.md) for commercial licensing.

"Migo" and the Migo logo are trademarks of the Migo Authors. The licence grants rights in the **software**, not in the name: you may fork the code, but not the name.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Before opening a pull request, check that the contract gates under `scripts/test-*-contract.sh` still pass; they encode invariants that ordinary tests do not cover.

## Acknowledgements

Migo builds on [Deno Core](https://github.com/denoland/deno_core), [V8](https://v8.dev/), [Tokio](https://tokio.rs/) and [Skia](https://skia.org/) (Ganesh GL backend + SkParagraph text layout). See [NOTICE](NOTICE) for the full dependency and licence list.

## Support

- Issues: https://github.com/minigame-labs/migo/issues
- Docs: https://github.com/minigame-labs/migo/wiki
