# MigoApplePerformancePlus

Conditional native-rendering lane. Content JavaScript and WebAssembly remain
in WebKit's WebContent process while Migo owns the bounded, validated render
ingress. Performance+ does not currently prove a no-V8 dependency closure: the
skeleton/product baseline may still link V8. A future release gate must prove
the final artifact and dependency tree do not link V8.

## What is decided, and why a probe does not get to decide it

These three axes were settled on capability grounds, not performance grounds,
and that distinction is the whole point: a probe measuring latency, jitter and
CPU cannot overturn a constraint that leaves one candidate *unable to express the
operation at all*. They are recorded here with their reasons so nobody reopens
them after reading a stale probe label.

**Content JavaScript runs in a Dedicated Worker; the Window agent is not a
symmetric alternative.** A Window agent's `[[CanBlock]]` is false, so
`Atomics.wait` is unavailable there. Every synchronous GPU readback this engine
must support -- `getImageData`, `readPixels`, `toDataURL` -- therefore has no
blocking primitive to build on in a Window agent: the operation *does not exist*
there rather than being slower. A Dedicated Worker's `[[CanBlock]]` is true. A
Worker also has no `document`/`window` to start with, which is what makes the
environment match the other five platforms instead of requiring DOM removal.

**The page origin is the literal loopback address `127.0.0.1`.** A custom URL
scheme is not a secure context, and promoting one to a secure context needs
private API, which cannot ship. No secure context means no `crossOriginIsolated`,
which means no `SharedArrayBuffer`, which removes the synchronous path above.
127.0.0.0/8 is potentially trustworthy under W3C Secure Contexts, and the literal
address is required because a `localhost` *hostname* does not resolve inside
`WKWebView`. A custom scheme is still worth probing as a **negative** control --
to watch `isSecureContext` actually report false -- not as a candidate.

**The frame clock is driven by the host through the control channel.** Not
because Workers lack `requestAnimationFrame`, but because
`requestAnimationFrame` in this engine is already host-fed on every shipped
platform: `engine/crates/runtime-v8/src/rendering/webgl/03_raf.js` awaits
`op_await_next_frame()`, which the host's vsync resolves. Host-driven is
therefore the isomorphic choice and leaves the embedded JavaScript unchanged --
only the op's landing point moves. A WebContent-side clock would introduce a
second timeline, and the phase error between the two is a problem this topology
does not otherwise have.

## What the host shape must satisfy, and what is still measured

Decided, because the OS enforces it:

- The `WKWebView` **must be attached to the view hierarchy**. Since iOS 16 an
  unattached one is killed.
- It must **not** be `isHidden`, zero-sized, or unattached.
- **Occlusion stops JavaScript execution**, so "covered by the CAMetalLayer" is
  not a usable shape. The target shape is attached and moved outside the visible
  area.

Still measured on the supported OS/device matrix, because these are
throttling-and-throughput questions rather than capability ones:

- Which attached-but-not-visible variant wins: off-screen, a 1x1 visible corner,
  or fully occluded. Whether the Worker gets throttled is what is being compared.
- Whether the loopback downlink needs a third leg at all. A custom-scheme
  streaming response runs its handler in the host process and saves a
  NetworkProcess hop, but it adds cross-origin and COEP complexity, so it is only
  worth building if a measurement shows the loopback downlink is the bottleneck.
- The one go/no-go: **per-frame IPC cost**. If it does not fit, this lane is not
  taken and the WebKit Full lane is the product.

The implementation must not infer any measured item from an old probe label or
from a simulator result. Correctness and isolation are measured before
performance, and a winner is judged on latency, p99 jitter, missed-vsync rate,
CPU, memory, backpressure, cancellation, lifecycle and occlusion/background
behaviour.

WebKit bug [191362](https://bugs.webkit.org/show_bug.cgi?id=191362) is
officially **RESOLVED FIXED**; it is not evidence that custom-scheme request
bodies fail. Any candidate still needs a real target-OS/device probe for its own
payload/API/body type, secure context, cross-origin isolation, CORS, copy count,
POST body delivery, cancellation, streaming and backpressure.

## Boundary and buffers

`SharedArrayBuffer` is allowed only as a small same-WebContent-process
Worker-to-Window synchronization mailbox for bounded control and reply state. It
must never carry frame bytes. Frame bytes move from Worker to Window through a
bounded transferable `ArrayBuffer` ping-pong or pool, and only then enter the app
transport.

The host side validates generation, sequence, lengths, credits and integrity
before materialization or GPU effects. Queues and allocations are bounded.

`webViewWebContentProcessDidTerminate:` must be implemented: retire the
generation, drop unacknowledged packets, rebuild the transport, and resume from a
signed checkpoint. **That recovery path has to carry `Canvas2DState` too.** Called
out because it is not hypothetical -- the same shape of defect, state that must
survive a context or process rebuild and was missed on one path, has broken four
times in this repository already (#48, PR #18, PR #19, PR #21).

The authoritative contracts are [`contracts/apple/`](../../../../contracts/apple): the
deployment floor with its per-lane minimums, and the profile-selection policy.
Each carries its reasoning inline and each has a gate that fails when a consumer
drifts from it.

The maintainer plan behind them is deliberately **not** in this repository --
`docs/` is gitignored, so a link into it resolves for nobody who clones this.
Everything above is either a specification fact, an OS-enforced constraint, or
checkable against this repository's own source, so none of it depends on a
document a cloner cannot read.
