# ProbeApp

G0 probes only. ProbeApp is never linked into a product. The authoritative
requirements are the [v5 implementation plan](../../../docs/superpowers/plans/2026-09-02-apple-performance-v5-final-implementation-plan.md)
and [v5 design](../../../docs/superpowers/specs/2026-09-02-apple-performance-v5-design.md).

G0 must select a winner, or an explicit capability-specific choice, only from
real evidence on the current minimum OS and representative devices. It must
not report a topology as successful or failed in advance.

| G0 probe arm | Alternatives to measure | Required evidence |
|---|---|---|
| JavaScript agent | Window; Dedicated Worker | Conformance, synchronous API behavior, memory, CPU, latency, and lifecycle under the same workload |
| WebKit-to-app transport | custom `WKURLSchemeHandler`; loopback WebSocket on literal `127.0.0.1`; hybrid only after directional-bottleneck evidence | Secure-context/isolation, CORS, actual payload/API/body type and POST body delivery, copies, cancellation, streaming, authentication, and bounded backpressure |
| Frame clock | feature-detected Worker rAF when available; Window rAF relay; host `CADisplayLink` relay | Input-to-present latency, p99 jitter, missed-vsync rate, CPU, and behavior during backgrounding/occlusion |
| `WKWebView` host shape | attached visible; transparent overlay; 1×1; off-screen; occluded | WebContent liveness, rendering correctness, lifecycle, memory, occlusion behavior, and App Review/public-API compliance |

The transport arm must record that WebKit bug
[191362](https://bugs.webkit.org/show_bug.cgi?id=191362) is officially
**RESOLVED FIXED**. It cannot be used as a pre-written custom-scheme failure.
The probe still tests the real payload and API/body type on each target
OS/device, including secure context, isolation, CORS, copying, body delivery,
cancellation, streaming, and backpressure. No arm is presumed to win.

**Agent-dependent handoff invariant.** If Worker wins, a bounded transferable
`ArrayBuffer` ping-pong/pool carries frame bytes to Window. A small same-
WebContent-process Worker↔Window `SharedArrayBuffer` mailbox carries
synchronization only and never carries frame bytes. If Window wins, there is no
Worker relay.

**Full matrix.** First isolate variables with a 32 KiB / 60 Hz run. Only
winning combinations run the full matrix: 4 KiB, 32 KiB, 256 KiB, 1 MiB, and
4 MiB payloads at 30/60/120 Hz where hardware allows; a steady 30-minute run;
burst traffic; two-credit backpressure; resource upload concurrent with frames;
small and maximum synchronous replies; cancellation; background/foreground;
occlusion; WebContent kill; memory pressure; and cold/warm start. Every packet
has a deterministic correctness hash and receipt.

The result records the complete matrix, selected capability key(s), raw traces,
and reasons for rejection. Any hybrid selection requires measured directional
bottleneck evidence. Product code remains unchanged until the G0 record is
reviewed.
