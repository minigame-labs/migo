# Migo Apple platform

SwiftPM package for iOS, iPadOS and macOS: three products (the WebKit
compatibility lane, the conditional Performance+ lane, and the macOS native V8
lane) over one shared renderer target.

The authoritative contracts are [`contracts/apple/`](../../contracts/apple): the
deployment floor with its per-lane minimums, and the profile-selection policy.
Each carries its reasoning inline and each has a gate that fails when a consumer
drifts from it.

The maintainer plan behind them is deliberately **not** in this repository --
`docs/` is gitignored, so a link into it resolves for nobody who clones this.
What matters for anyone reading the code is in the contracts above and in the
comments here.

## Status

Not a skeleton any more, and not a product either. What separates the two halves
is whether a compiler has seen it, so that is what this section reports.

| | Where it is checked |
|---|---|
| `core/` (`MigoAppleCore`) | built, tested and cross-compiled for `aarch64-apple-ios` on every pull request, on an Apple silicon runner |
| `Sources/MigoAppleRenderer` | compiled for iOS, iOS Simulator and macOS by `.github/workflows/apple-sdk.yml`, and on the macOS leg linked into a test binary and executed on Apple silicon -- the only place the C ABI has ever run on an Apple machine |
| `Sources/MigoAppleWebKit`, `Sources/MigoMacV8` | placeholders; they compile, and they do nothing |
| `WebContent/PerformancePlus` | its encoder runs against the same golden corpus as the Rust one, under node, on every pull request |
| `ProbeApp/` | a README. No sources exist, and none should until there is a device to run them on |

Nothing here has run on an iPhone. The Performance+ topology -- agent,
transport, frame clock, host shape -- is unselected, and the G0 probe evidence
selects it; see the table below.

The product baseline also does not prove that Performance+ is V8-free. The
Cargo half of that claim is checked on Linux every pull request and the archive
half now reads a real symbol table produced by Apple's toolchain, but the claim
that belongs to a *release* is about a shipped artifact, and there is no Apple
release yet.

The first Mac task is the G0 probe contract. It must measure the unresolved
topology axes below on the current minimum OS and representative devices;
ProbeApp evidence, not this skeleton, selects the product choices.

| G0 axis | Candidates that must be measured |
|---|---|
| JavaScript agent | Window vs Dedicated Worker |
| WebKit-to-app transport | custom scheme vs loopback WebSocket vs hybrid; hybrid only after directional-bottleneck evidence |
| frame clock | Worker `requestAnimationFrame` when feature-detected vs Window rAF relay vs `CADisplayLink` relay |
| `WKWebView` host shape | attached visible vs transparent overlay vs 1×1 vs off-screen vs occluded |

Correctness and isolation precede measurements of input-to-present latency, p99
jitter, missed-vsync rate, CPU, memory, cancellation, backpressure, lifecycle,
and background/occlusion behavior. No candidate may be called a winner in
advance.

WebKit bug [191362](https://bugs.webkit.org/show_bug.cgi?id=191362) is
officially **RESOLVED FIXED** and is not a pre-written custom-scheme failure.
Probe the actual payload/API/body type on each target OS/device for
secure-context/isolation, CORS, copy count, POST body delivery, cancellation,
streaming, and backpressure.

If G0 selects a Dedicated Worker, `SharedArrayBuffer` is only a small
same-WebContent-process Worker↔Window synchronization mailbox; it never carries
frame bytes. Frame bytes first use a bounded transferable `ArrayBuffer`
ping-pong/pool to Window and then the selected app transport. If G0 selects
Window, there is no Worker relay.

## Three products, three JavaScript execution models

| Product | Where content JS runs | Renderer | Ships in v1 |
|---|---|---|---|
| `MigoAppleWebKit` | WebContent, full web platform | WebKit | yes |
| `MigoApplePerformancePlus` | WebContent, Window or Dedicated Worker selected by G0 | this process: Skia + ANGLE/Metal | conditional |
| `MigoMacV8` | this process, V8 with JIT | this process: Skia + ANGLE/Metal | yes, macOS only |

`MigoAppleRenderer` is shared by the two native-rendering lanes and is
deliberately **not** a product. `MigoAppleWebKit` deliberately does not depend
on it: a host that asked for the compatibility baseline should not be linking a
renderer it will never drive.

Native JSC is not in v1. Reopen it only if credible lightweight content
produces a measured low-memory use case; until then, `MigoAppleWebKit` remains
the compatibility baseline for that content. This is a future evidence gate,
not a permanent architectural veto.

## Why iOS needs a native surface at all

The earlier C ABI candidate declared no iOS surface descriptor, on the
assumption that iOS would be a `WKWebView` container where WebKit owns the
drawing surface. That assumption inherited a claim -- "iOS has no performance
path" -- which turned out to be about in-process engines only. WebKit's
WebContent process is spawned by the system with the JIT entitlement; the
boundary is drawn around the process, not around the engine. So content
JavaScript can have JIT *and* Migo can own the renderer, as long as a frame's
worth of drawing commands crosses one process boundary once per frame.

That is the Performance+ lane, and it needs a host-owned `CAMetalLayer`, which
is why `include/migo/platform/ios.h` exists.

## Layout

```
platforms/apple/
  core/                      the engine-free package: no binary target, no dependencies
    Package.swift              so it resolves and builds with nothing fetched and nothing built
    Sources/MigoAppleCore/     profile policy, deployment floors, lifecycle, permissions, metrics
    Tests/
  Package.swift              the shipping package; floor values derived from
                             contracts/apple/deployment-floor.json
  Frameworks/                generated: MigoEngine.xcframework (gitignored)
  Sources/
    MigoAppleRenderer/       internal: CAMetalLayer, display link, surface attach
    MigoAppleWebKit/         lane 1
    MigoApplePerformancePlus/ lane 2: transport, FrameIngress bridge, host view
      Resources/               generated: the WebContent bundle (gitignored)
    MigoMacV8/               lane 3
  Tests/
  WebContent/PerformancePlus/ source of the bundle that runs inside WebContent
  ProbeApp/                  G0 probes only; never linked into a product
```

**Two packages, and the split is load-bearing.** The shipping package declares a
binary target for an xcframework that `scripts/build-apple-sdk.sh` produces, so
it does not resolve until that script has run on a Mac -- which meant, for the
whole life of the skeleton, that *no* Swift here was ever compiled, including
the files that mirror `contracts/apple/*.json` and whose gates compare only
their text. `core/` is the half that needs no engine, so it is built and tested
on every pull request. `scripts/test-apple-swift-core-engine-free.sh` keeps it
engine-free and `scripts/test-apple-shipping-package-contract.sh` keeps the
shipping package consuming the artifact it declares.

`WebContent/PerformancePlus` is Apple product source, not a cross-platform
adapter, so it does not live in `adapter/`: it depends on WebKit bootstrap
order, on the topology and transport selected by G0, and on the Apple release
receipt.

`ProbeApp` never enters a release target. It exists to answer G0 and to keep
answering it when a new iOS version ships.

## Building

`swift build` fails until the engine binary exists:

```sh
bash scripts/build-apple-sdk.sh --platform macos --configuration Debug
```

That is deliberate. The alternative -- `unsafeFlags` pointing at a local
`libmigo.a` -- would make this package impossible for anyone else to depend on,
which SwiftPM enforces by refusing such packages as dependencies.
