# Performance+ WebContent producer

Source for the bundle that runs inside WebKit's WebContent process. Bundled and
minified by `scripts/build-apple-sdk.sh` into
`../../Sources/MigoApplePerformancePlus/Resources/`.

Two execution contexts, and the split is the design:

**Worker** — the game's JavaScript and WebAssembly, plus Migo's existing
embedded bundle. The Worker global has no `document` and no `window`, which is
already what Migo provides on Android and desktop, so the 132 embedded JS files
move here without an environment shim. Their `Deno.core.ops` calls resolve
through a generated shim instead of deno_core, into one of four dispositions:
answered from shadow state, appended to the outgoing binary command stream,
sent as a control-channel promise, or -- for the few calls that must genuinely
block -- a `SharedArrayBuffer` round trip parked on `Atomics.wait`.

**Page main thread** — the relay, and nothing else. It owns the reply socket,
writes results back into the SAB and calls `Atomics.notify`. It never runs game
logic, so a busy frame cannot delay a synchronous reply.

Not a cross-platform adapter, and deliberately not in `adapter/`: it depends on
WebKit bootstrap order, on the transport M0-P3 selects, and on the Apple
release receipt.

Tests here run under node, with no device and no simulator, against the same
golden wire corpus the Rust validator uses. Two encoders that agree with each
other but not with a fixed corpus is the failure mode a shared corpus prevents.
