# Performance+ WebContent producer

Source for the bundle that runs inside WebKit's WebContent process. Bundled
and minified by `scripts/build-apple-sdk.sh` into
`../../Sources/MigoApplePerformancePlus/Resources/`.

G0 has not selected an execution topology. The producer must support probing
both a Window agent and a Dedicated Worker agent, and must not assume that a
Worker has or lacks rAF: feature-detect Worker
`requestAnimationFrame` when available, and measure it against Window rAF
relay and host `CADisplayLink` relay.

When a Worker is selected, game JavaScript/Wasm runs there and the Window is a
relay. A small `SharedArrayBuffer` may be used only for same-WebContent
Worker↔Window synchronization (control/reply mailbox); it never carries frame
bytes. Frame bytes use a bounded transferable `ArrayBuffer` ping-pong/pool to
the Window before entering the transport selected by G0. When Window is
selected, there is no Worker relay.

The transport probe compares custom scheme, loopback WebSocket, and a hybrid
only after directional-bottleneck evidence. WebKit bug
[191362](https://bugs.webkit.org/show_bug.cgi?id=191362) is officially
**RESOLVED FIXED**, so it is not a pre-written POST-body failure. The probe
must exercise the actual payload/API/body type on each target OS/device and
record secure-context/isolation, CORS, copy count, body delivery,
cancellation, streaming, and backpressure.

This WebContent bundle does not make the current Performance+ skeleton
V8-free. The current dependency chain may still link V8; the future release
gate must prove the final Performance+ artifact and dependency closure do not.

Not a cross-platform adapter: bootstrap order, selected topology/transport, and
the Apple release receipt are part of this source's contract. Tests run under
node, with no device or simulator, against the same golden wire corpus used by
the Rust validator; device claims require the G0 evidence.
