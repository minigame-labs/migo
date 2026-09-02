# MigoAppleRenderer (internal)

Owns the single `CAMetalLayer` a Session presents to, the display link that
paces it, and the Surface attach/update/retire handshake against the engine's C
ABI.

Not a product, and not a supported dependency: it is shared by
`MigoApplePerformancePlus` and `MigoMacV8`, and depending on it directly would
let a host acquire the renderer without the lane that knows how to drive it.

Rules that are not negotiable here, and the reason each one exists:

- **One layer per Session.** Offscreen canvases render into FBOs and are
  composited; they do not each get a layer. A layer per canvas multiplies
  drawables, caches, contexts and compositor memory.
- **Acquire the drawable last.** Everything that can be encoded before
  `nextDrawable` is encoded first, and the drawable is released as soon as the
  command buffer is committed. Holding one drains the pool and blocks the next
  `nextDrawable` call.
- **Never `waitUntilCompleted` on a foreground frame.** Synchronous readback
  goes through its own barrier with a deadline.
- **Carry Canvas2D state across every surface rebuild.** Backgrounding,
  rotation, a `contentsScale` change and a WebContent restart all rebuild the
  surface. This invariant has been broken four separate times in this repo
  (#48, PR#18, PR#19, PR#21); the fourth was an in-place resize, which is why
  the rule is phrased as "every rebuild path" and not "drop and recreate".
