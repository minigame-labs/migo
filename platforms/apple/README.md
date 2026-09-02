# Migo Apple platform

SwiftPM package for iOS, iPadOS and macOS. The authoritative design and the
milestone plan are in `docs/apple-final-implementation-plan.md` (maintainer
notes, not shipped).

## Status: skeleton

Nothing in this directory has been compiled. There is no macOS machine in the
development environment that produced it, so every Swift file here is a
declaration of intent, not a verified implementation. Treat it accordingly:

- What **is** verified, on Linux, by gates that have been shown to turn red:
  the C ABI surface descriptors for both Apple platforms
  (`engine/crates/capi-abi`, `tests/c_abi/platform_contract.c`), and the
  deployment-floor contract (`scripts/test-apple-deployment-floor-contract.sh`).
- What is **not** verified: all of `Sources/`, `WebContent/`, `ProbeApp/`.

The first task for whoever has a Mac is not to add features here. It is M0 in
the plan -- seven probes, none of which needs any of this code -- because five
of the load-bearing assumptions behind the architecture are still unmeasured,
and two of them would change the architecture if they came back false.

## Three products, three JavaScript execution models

| Product | Where content JS runs | Renderer | Ships in v1 |
|---|---|---|---|
| `MigoAppleWebKit` | WebContent, full web platform | WebKit | yes |
| `MigoApplePerformancePlus` | WebContent, in a Worker, no DOM | this process: Skia + ANGLE/Metal | yes |
| `MigoMacV8` | this process, V8 with JIT | this process: Skia + ANGLE/Metal | yes, macOS only |

`MigoAppleRenderer` is shared by the two native-rendering lanes and is
deliberately **not** a product. `MigoAppleWebKit` deliberately does not depend
on it: a host that asked for the compatibility baseline should not be linking a
renderer it will never drive.

There is no fourth product. In-process JavaScriptCore was evaluated and
rejected: the JIT boundary on iOS is the process, so an in-process engine is
the interpreted-only tier that WeChat measured at 13 FPS against 49 for the
same content out of process. Its only wins are package size and one fewer
process, and `MigoAppleWebKit` already serves the light content that would have
been its audience at zero additional cost.

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
  ProbeApp/                  M0 probes only; never linked into a product
```

`WebContent/PerformancePlus` is Apple product source, not a cross-platform
adapter, so it does not live in `adapter/`: it depends on WebKit bootstrap
order, on the transport chosen by M0-P3, and on the Apple release receipt.

`ProbeApp` never enters a release target. It exists to answer M0 and to keep
answering it when a new iOS version ships.

## Building

`swift build` fails until the engine binary exists:

```sh
bash scripts/build-apple-sdk.sh --platform macos --configuration Debug
```

That is deliberate. The alternative -- `unsafeFlags` pointing at a local
`libmigo.a` -- would make this package impossible for anyone else to depend on,
which SwiftPM enforces by refusing such packages as dependencies.
