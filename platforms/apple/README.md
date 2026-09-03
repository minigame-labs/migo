# Migo Apple platform

SwiftPM package for iOS, iPadOS and macOS. The only authoritative sources are
the [v5 implementation plan](../../docs/superpowers/plans/2026-09-02-apple-performance-v5-final-implementation-plan.md)
and the [v5 design](../../docs/superpowers/specs/2026-09-02-apple-performance-v5-design.md).
The short compatibility pointer at `docs/apple-final-implementation-plan.md`
must remain a pointer, not a second plan.

## Status: skeleton

Nothing in this directory has been compiled on Apple hardware. The Swift,
WebContent, and ProbeApp files are declarations of intent, not verified
implementations. The current skeleton and product baseline also do not prove
that Performance+ is V8-free: the current dependency chain may still link V8.
The future release gate must prove that the final Performance+ artifact and
dependency closure do not link V8.

What is verified on Linux is limited to the existing C ABI and deployment-floor
gates. All of `Sources/`, `WebContent/`, and `ProbeApp/` remain unverified on
Apple hardware.

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
  Package.swift              floor values derived from contracts/apple/deployment-floor.json
  Sources/
    MigoAppleCore/           profile policy, lifecycle, permissions, metrics
    MigoAppleRenderer/       internal: CAMetalLayer, display link, surface attach
    MigoAppleWebKit/         lane 1
    MigoApplePerformancePlus/ lane 2: transport, FrameIngress bridge, host view
      Resources/               generated: the WebContent bundle (gitignored)
    MigoMacV8/               lane 3
  WebContent/PerformancePlus/ source of the bundle that runs inside WebContent
  Tests/
  ProbeApp/                  G0 probes only; never linked into a product
```

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
