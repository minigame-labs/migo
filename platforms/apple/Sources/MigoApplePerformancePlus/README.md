# MigoApplePerformancePlus

Conditional native-rendering lane. Content JavaScript and WebAssembly remain
in WebKit's WebContent process while Migo owns the bounded, validated render
ingress. Performance+ does not currently prove a no-V8 dependency closure: the
skeleton/product baseline may still link V8. A future release gate must prove
the final artifact and dependency tree do not link V8.

## G0 is still a measured choice

The following axes are unresolved until the ProbeApp evidence selects a winner
on the supported OS/device matrix:

- JavaScript agent: Window vs Dedicated Worker.
- App transport: custom scheme vs loopback WebSocket; hybrid is considered
  only after evidence of a directional bottleneck.
- Frame clock: feature-detected Worker rAF when available, Window rAF relay, or
  `CADisplayLink` relay.
- `WKWebView` host shape: attached visible, transparent overlay, 1×1,
  off-screen, or occluded.

The implementation must not infer any of these from an old probe label or from a
simulator result. Correctness/isolation is measured before performance, and
the winner is judged by latency, p99 jitter, missed-vsync rate, CPU, memory,
backpressure, cancellation, lifecycle, and occlusion/background behavior.

WebKit bug [191362](https://bugs.webkit.org/show_bug.cgi?id=191362) is
officially **RESOLVED FIXED**; it is not evidence that custom-scheme request
bodies fail. Each candidate still needs a real target-OS/device probe for the
actual payload/API/body type, secure context, cross-origin isolation, CORS,
copy count, POST body delivery, cancellation, streaming, and backpressure.

## Boundary and buffers

If G0 selects Dedicated Worker, `SharedArrayBuffer` is allowed only as a small
same-WebContent-process Worker↔Window synchronization mailbox for bounded
control/reply state. It must never carry frame bytes. Frame bytes first move
from Worker to Window through a bounded transferable `ArrayBuffer` ping-pong or
pool, and only then enter the G0-selected app transport. If G0 selects Window,
there is no Worker relay and no Worker mailbox.

The host side validates generation, sequence, lengths, credits, and integrity
before materialization or GPU effects. Queues and allocations are bounded.

The final product architecture and release gates are defined by the [v5
implementation plan](../../../../docs/superpowers/plans/2026-09-02-apple-performance-v5-final-implementation-plan.md)
and [v5 design](../../../../docs/superpowers/specs/2026-09-02-apple-performance-v5-design.md).
