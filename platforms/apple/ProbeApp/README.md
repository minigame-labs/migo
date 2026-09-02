# ProbeApp

M0 only. Never linked into a product, and the packaging gate asserts that.

Each probe settles one load-bearing assumption from
`docs/apple-final-implementation-plan.md` section 10. A probe that cannot run
leaves its assumption unsettled; it does not get to report a pass.

| Probe | Settles | Passes when |
|---|---|---|
| P0 loopback origin | A6 | the 127.0.0.1 page and every subresource load on iOS 15.2 / 17 / 18 / 26 hardware, with no local-network permission prompt |
| P0.5 WebView host shape | A4 | a busy Worker keeps full speed for 10 minutes in whichever of offscreen / 1x1 visible / occluded is chosen |
| P1 isolation and SAB | A7, A15 | `crossOriginIsolated` is true and an `Atomics.wait` round trip completes, with its latency recorded |
| P2 ANGLE-Metal | A9 | `glReadPixels` verifies the pixels. Looking at the screen is not a pass |
| P3 transport | A8 | one real 32 KiB WebGL frame, p50 and p99, one way plus acknowledgement, against a WebKit Full baseline -- this is the decision point for the whole lane |
| P4 Worker environment | A10 | the existing embedded bundle runs a conformance subset inside a Worker |
| P5 cold start | A14 | WebContent spawn plus bundle load, measured the same way Android first-frame is |

P3 also runs the custom-scheme arm, and is expected to fail there. That failure
is the point: the arm is kept to produce the evidence that a custom scheme is
not a secure context and loses the POST body, rather than leaving a rejected
option rejected only in prose. A gate nobody has seen fail is a gate nobody
should trust, and the same is true of a rejected design.
