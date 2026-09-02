# MigoApplePerformancePlus

Lane 2, and the reason the Apple work is worth doing: content JavaScript and
WebAssembly run with JIT, and Migo still owns the renderer.

```
WebContent process                    this process
  Worker (game JS/WASM, no DOM)
    command stream encoder    --WS-->   validator
    Atomics.wait for sync calls          FrameIngress
  main thread: relay                     RenderServer
                                         Skia + ANGLE/Metal -> CAMetalLayer
                              <--tick--  CADisplayLink (+ batched input)
```

Four decisions here are load-bearing, and each one was reached by rejecting an
alternative that looks reasonable until it is followed through:

**Content runs in a Worker, not on the page's main thread.** A Worker global
has no `document` and no `window`, which is already the environment Migo
provides on Android and desktop and already the mini-game platform contract, so
nothing has to be deleted to get there. It also has `[[CanBlock]]`, so
`Atomics.wait` is legal -- and that is the only thing that makes synchronous
`getImageData`/`readPixels`/`toDataURL` work at all across a process boundary.
On the page's main thread `Atomics.wait` throws, which forces either rewriting
the content or declaring those calls unsupported.

**The page is served from 127.0.0.1, not a custom scheme.** A custom scheme is
not a secure context (making it one needs private API, which cannot ship), and
without a secure context there is no `crossOriginIsolated` and therefore no
`SharedArrayBuffer`. Independently, WebKit loses the HTTP body on POST to a
custom scheme, which removes the binary upstream channel as well. The loopback
origin, with COOP `same-origin` and COEP `require-corp`, buys the secure
context, SharedArrayBuffer, a binary channel and content delivery in one
decision. Bind the literal `127.0.0.1`; the `localhost` hostname does not
connect from WKWebView.

**Frames are driven by the host's `CADisplayLink`, not by rAF.** A Worker has
no `requestAnimationFrame` at all -- and Migo's own `requestAnimationFrame` was
never the browser's: it is `await op_await_next_frame()`, already fed by host
vsync on every other platform. Moving it here changes where the op resolves and
nothing else, and it leaves one timeline instead of two to reconcile.

**Input is native and rides the same downstream tick.** The WebView is attached
but out of the visible area, and the content has no DOM, so it can neither
receive nor use touches. Batching input onto the frame tick costs no extra IPC.

The WebView must stay attached to the view hierarchy. iOS kills unattached
`WKWebView` instances, and occluded ones stop executing JavaScript; attached
but moved off-screen is the shape that has been observed to keep running, and
M0-P0.5 is what decides between the candidates.
