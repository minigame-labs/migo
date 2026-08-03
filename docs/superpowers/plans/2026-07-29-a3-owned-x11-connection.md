# Owned X11 Render Connection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove Migo's undocumented `XInitThreads` dependency by rendering
through a Migo-owned X11 connection that is never concurrently accessed from
multiple Migo threads.

**Architecture:** A Linux-only `X11RenderConnection` resolves the caller's X11
server on the attach thread, opens and owns a second connection through a
narrow dynamically loaded Xlib boundary, and exposes only an opaque render
display token. A Session-scoped platform context reuses that exact connection
for later windows on the same server. Because the texture upload worker shares
an EGLDisplay with the render thread, EGL providers declare an explicit
concurrency policy and X11 disables that worker; the other shipping providers
retain it.

**Tech Stack:** Rust 2024, dynamic Xlib loading, Linux socket endpoint identity,
EGL provider injection, C ABI Session lifecycle, Xvfb, C host integration.

---

## Invariants

- The caller's `Display*` is touched only synchronously by attach on the caller
  thread. It is never stored in a `Send` or `Sync` production type.
- The host continues to own its event connection and X11 `Window`.
- EGL sees only the Migo-owned render connection.
- The owned connection is opened before a Host is spawned, is reused for
  compatible reattachments, and is closed exactly once after its last EGL
  owner has terminated.
- X11 uses no background shared-context upload worker. Android system EGL,
  Linux offscreen/Wayland EGL, and Windows ANGLE keep the existing worker.
- Reattachment to another X11 server fails synchronously before Surface lease
  creation or command enqueue. A same-server replacement reuses the exact
  original `PlatformIdentity`.
- No host, example, or public document requires `XInitThreads`.

## Task 1: Make EGL Concurrency An Explicit Provider Contract

**Files:**
- Modify: `engine/crates/graphics/src/egl_platform.rs`
- Modify: `engine/crates/graphics/src/upload_thread.rs`
- Modify: `engine/crates/graphics/src/canvas/manager/egl_ops.rs`
- Modify: `engine/crates/platform/src/android/presenter.rs`
- Modify: `engine/crates/platform/src/linux/presenter.rs`
- Modify: `engine/crates/platform/src/windows/presenter.rs`

- [x] **Step 1: Add a failing upload-policy test**

Add `EglConcurrency::{RenderThreadOnly, SharedContexts}` and make the test
provider used by `UploadThreadHandle::try_spawn` return
`RenderThreadOnly`. Assert that the function returns `None` before loading EGL,
creating an EGL context, or spawning `Migo-Upload`.

Run:

```bash
cd engine
cargo test -p migo-graphics upload_thread::tests::render_thread_only_provider_never_spawns_upload_worker --lib --locked --offline
```

Expected: FAIL because `EglProvider` has no concurrency contract.

- [x] **Step 2: Require every provider to declare concurrency**

Add this required method to `EglProvider`:

```rust
fn concurrency(&self) -> EglConcurrency;
```

Do not provide a permissive default. Every shipping and injected provider must
choose deliberately. Return `RenderThreadOnly` only for Linux X11; return
`SharedContexts` for Android system EGL, Linux offscreen/Wayland, and Windows
ANGLE. In `UploadThreadHandle::try_spawn`, return `None` before all EGL work
when the provider is `RenderThreadOnly`.

- [ ] **Step 3: Verify there is no hidden X11 upload path**

Run:

```bash
cd engine
cargo test -p migo-graphics upload_thread::tests --lib --locked --offline
cargo check -p migo-graphics -p migo-platform --all-targets --locked --offline
```

Expected: PASS, and every `impl EglProvider` implements `concurrency`.

## Task 2: Own One Narrow X11 Render Connection

**Files:**
- Create: `engine/crates/platform/src/linux/x11_connection.rs`
- Modify: `engine/crates/platform/src/linux/mod.rs`
- Modify: `engine/crates/platform/src/linux/presenter.rs`
- Modify: `engine/crates/platform/Cargo.toml`

- [x] **Step 1: Add failing connection-owner tests**

Use an injected `X11Api` in unit tests to prove:

- attach copies `XDisplayString` while the caller display is borrowed;
- `XOpenDisplay` is called once with those copied bytes;
- a null `XOpenDisplay` result is an error with the requested display name;
- host and owned connections must resolve to the same server endpoint;
- `Drop` calls `XCloseDisplay` exactly once;
- the host display is never closed;
- no Xlib function is called after the dynamic library owner drops.

Run:

```bash
cd engine
cargo test -p migo-platform x11_connection::tests --lib --locked --offline
```

Expected: FAIL because there is no owned connection type.

- [x] **Step 2: Implement the dynamic Xlib boundary**

Load only `XDisplayString`, `XOpenDisplay`, `XCloseDisplay`, and
`XConnectionNumber` from `libX11.so.6` with `libloading`. Copy the display name
into a `CString` before opening the owned connection. Identify the server from
the connection file descriptor:

- `AF_UNIX`: use Linux `SO_PEERCRED`;
- `AF_INET`/`AF_INET6`: use the exact peer socket address;
- unsupported address families or failed peer inspection: fail attach.

Declare `libc = "0.2"` directly under the Linux target dependencies in
`engine/crates/platform/Cargo.toml`; do not rely on another crate placing it in
the dependency graph.

Store the endpoint identity and fd with the owned display. Before reusing an
existing context, use a zero-timeout `poll` and reject `POLLERR`, `POLLHUP`, or
`POLLNVAL`, then compare the candidate host connection's exact endpoint.
Neither operation consumes X11 bytes or calls Xlib on the owned display.

`X11RenderConnection` owns the dynamic API and non-null display pointer. Its
`Drop` closes the display before the API owner can drop. The only unsafe
`Send`/`Sync` justification is that EGL receives this connection exclusively on
the render thread and Task 1 prohibits a second Migo EGL thread for X11.

- [x] **Step 3: Bind presenter identity and surfaces to the owner**

Add `LinuxX11Context` with these operations:

```rust
pub fn open(host_display: NonNull<c_void>) -> EngineResult<Self>;
pub fn supports_host_display(&self, host_display: NonNull<c_void>) -> EngineResult<()>;
pub fn graphics_platform(&self) -> GraphicsPlatform;
pub fn surface(&self, window: c_ulong, width: u32, height: u32) -> SurfaceRef;
```

The provider, factory, `LinuxX11Surface`, and
`LinuxX11PreparedSurface` hold clones of the owned connection. Provider and
factory identities use the owned `Display*`, never the caller's pointer. The
factory accepts only a surface holding the same `Arc` owner. Remove the
host-display pointer from all render-thread surface types and from their unsafe
comments.

- [ ] **Step 4: Add a real Xvfb connection test**

Add an ignored native test that opens one host connection under `DISPLAY`,
constructs `LinuxX11Context`, proves the host and render pointers differ, proves
the context accepts the host connection, captures the owned fd through a
test-only accessor, and verifies `fcntl(F_GETFD)` reports `EBADF` after all
context/platform/surface owners drop.

Run it explicitly so absence of a display cannot become a passing skip:

```bash
cd engine
xvfb-run -a cargo test -p migo-platform \
  linux::x11_connection::tests::native_owned_connection_round_trip \
  --lib --locked --offline -- --ignored --exact
```

Expected: PASS.

## Task 3: Reuse The Connection Across C ABI Reattachment

**Files:**
- Modify: `engine/crates/capi/src/platform/mod.rs`
- Modify: `engine/crates/capi/src/platform/android.rs`
- Modify: `engine/crates/capi/src/platform/linux.rs`
- Modify: `engine/crates/capi/src/platform/windows.rs`
- Modify: `engine/crates/capi/src/platform/unsupported.rs`
- Modify: `engine/crates/capi/src/lib.rs`
- Modify: `engine/crates/capi/src/surface.rs`
- Modify: `engine/crates/capi/src/test_support.rs`

- [ ] **Step 1: Add failing platform-context tests**

Add tests proving:

- a cold X11 attach opens one render connection;
- a same-server later attach reuses it and yields equal
  `PlatformIdentity`;
- a different server returns `MIGO_ERROR_INVALID_STATE`;
- rejection occurs before `lease_surface_tracked`;
- Session state cannot contain Host, identity, or platform context without the
  other two;
- destroy clears the context beside the Host owner without closing a
  connection still retained by the exiting Host.

Run:

```bash
cd engine
cargo test -p migo-capi platform_context --lib --locked --offline
```

Expected: FAIL because Session state retains no platform construction context.

- [x] **Step 2: Add one target-specific `PlatformContext` contract**

Each C ABI platform module exports a cloneable `PlatformContext` plus:

```rust
pub(crate) fn build_target(
    descriptor: SurfaceDescriptorRef,
    existing: Option<&PlatformContext>,
) -> Result<(SurfaceRef, GraphicsPlatform, PlatformTarget, PlatformContext), MigoResult>;
```

Android and Windows contexts retain their immutable graphics platform. Linux
uses `LinuxX11Context` for X11 and a fixed display/platform pair for Wayland.
The unsupported module returns `MIGO_ERROR_UNSUPPORTED_PLATFORM`.

Store `Option<PlatformContext>` beside `HostThread` and `PlatformIdentity`.
Capture a clone while starting the attach transition, build against it outside
the Session lock, run the existing generic identity comparison, and commit all
three cold-start fields in one lock acquisition. On reattach, require the
returned context identity to equal the stored identity and do not replace the
stored context.

- [x] **Step 3: Preserve rollback and destruction ordering**

Every cold-start failure after connection creation must shut down and join the
new Host before its last platform context can drop. Session destroy removes its
context clone while transferring the Host owner to the Engine; the Host's
provider/factory clone keeps the connection alive until EGL termination and
worker exit. Engine destroy remains the final observable connection-close
barrier.

- [ ] **Step 4: Verify C ABI ordering**

Run:

```bash
cd engine
cargo test -p migo-capi platform_context --lib --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh --core
bash scripts/test-surface-attachment-contract.sh
```

Expected: PASS. Source order must show platform-context/server validation before
the first lease and before `UpdateSurface` enqueue.

## Task 4: Remove The Public And Example Precondition

**Files:**
- Modify: `include/migo/platform/x11.h`
- Modify: `include/migo/surface.h`
- Modify: `include/migo/README.md`
- Modify: `engine/tools/player/src/x11_window.rs`
- Modify: `engine/tools/player/src/main.rs`
- Modify: `tests/c_host/linux/main.c`
- Modify: `scripts/test-surface-attachment-contract.sh`
- Create: `scripts/test-x11-owned-connection-contract.sh`

- [x] **Step 1: Add a failing repository contract**

The new script must fail unless:

- production X11 platform code contains the owner and `XOpenDisplay` /
  `XCloseDisplay` boundary;
- `XInitThreads` is absent from runtime, player, C host, and public headers;
- the upload-policy test names X11 as render-thread-only;
- C ABI attach passes an existing platform context before any lease;
- public docs state the host `Display*` is borrowed synchronously while the
  host-owned `Window` remains valid through `RELEASED`.

Run:

```bash
bash scripts/test-x11-owned-connection-contract.sh
```

Expected: FAIL before implementation.

- [x] **Step 2: Update public ownership language**

Document that attach synchronously resolves the server and opens a private
render connection. Migo never closes or dispatches events on the host
connection, never destroys the host window, and requires no `XInitThreads`.
Same-server window replacement reuses the Session's render connection;
different-server replacement returns `MIGO_ERROR_INVALID_STATE`.

- [x] **Step 3: Remove host-side initialization**

Delete `XInitThreads` from the player and public C host. Their event loops
continue using only the host-owned display on their own thread. Construct the
surface/platform through `LinuxX11Context` so the Rust player exercises the
shipping owner path rather than an internal shortcut.

- [ ] **Step 4: Run the real C host under Xvfb**

```bash
xvfb-run -a bash scripts/dev-run-c-host.sh
```

Expected: the C host reaches first frame, detaches, destroys Session and Engine,
and exits without `XInitThreads`.

## Task 5: Close A3

- [ ] **Step 1: Run broad host checks**

```bash
cd engine
cargo fmt --all -- --check
cargo test -p migo-graphics --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cargo test -p migo-capi --lib --locked --offline
cargo check -p migo-graphics -p migo-platform -p migo-capi -p migo-player \
  --all-targets --locked --offline
cd ..
bash scripts/test-x11-owned-connection-contract.sh
bash scripts/test-c-abi-surface-candidate.sh
bash scripts/test-surface-attachment-contract.sh
```

- [ ] **Step 2: Inspect all X11 ownership and thread boundaries**

```bash
rg -n "XInitThreads|XOpenDisplay|XCloseDisplay|X11RenderConnection|LinuxX11Context" \
  engine include tests scripts
rg -n "unsafe impl (Send|Sync)" engine/crates/platform/src/linux
rg -n "EglConcurrency|Migo-Upload" engine/crates
```

Expected: no undocumented precondition, no host `Display*` retained in a
cross-thread type, one owned connection close path, and no X11 upload worker.

- [ ] **Step 3: Update ledgers and commit**

Mark A3 complete only after the fake-boundary tests, Xvfb native connection
test, real C host, product-profile checks, and broad linked tests all pass.
Commit verified implementation and evidence locally; do not push.
