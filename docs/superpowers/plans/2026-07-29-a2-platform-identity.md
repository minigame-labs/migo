# Immutable Graphics Platform Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject a Surface reattachment synchronously unless the existing Host
can present it through the exact same graphics platform domain.

**Architecture:** `GraphicsPlatform` owns an immutable, process-local
`PlatformIdentity` supplied independently by its EGL provider and surface
factory. Construction fails when either the backend or native display/device
domain differs. A C ABI Session stores the identity committed with its first
Host and compares every later candidate before leasing a Surface or enqueueing a
render command.

**Tech Stack:** Rust 2024, EGL provider/factory injection, C ABI lifecycle,
target-independent unit tests, Linux/Android/Windows presenter contract tests.

---

## Task 1: Define And Validate `PlatformIdentity`

**Files:**
- Modify: `engine/crates/graphics/src/egl_platform.rs`
- Modify: `engine/crates/graphics/src/upload_thread.rs`
- Modify: `engine/crates/graphics/src/canvas/manager/egl_ops.rs`

- [x] **Step 1: Add failing identity tests**

Add tests proving:

- backend-equal identities from different native domains are unequal;
- provider/factory construction rejects an identity mismatch before EGL load;
- a constructed `GraphicsPlatform` exposes one immutable identity;
- prepared-surface validation still rejects a different backend.

Run:

```bash
cd engine
cargo test -p migo-graphics egl_platform::tests --lib --locked --offline
```

Expected: FAIL because `GraphicsPlatform` exposes only `GraphicsBackendId`.

- [x] **Step 2: Implement process-local identity**

Add an opaque, clone/copy, equality-comparable `PlatformIdentity` containing:

- the existing `GraphicsBackendId`;
- a private `TypeId` domain marker;
- a process-local native instance token.

Provide constructors that require a private concrete marker type. Add
`platform_identity()` to `EglProvider` and `EglSurfaceFactory`, validate exact
equality in `GraphicsPlatform::try_new`, store it once, and expose a read-only
accessor. Do not serialize pointers, hash identities, allocate on comparisons,
or add any draw/present hot-path work.

- [ ] **Step 3: Verify graphics identity**

```bash
cd engine
cargo test -p migo-graphics egl_platform::tests --lib --locked --offline
cargo check -p migo-graphics --all-targets --locked --offline
```

Expected: PASS.

## Task 2: Give Every Shipping Presenter An Exact Identity

**Files:**
- Modify: `engine/crates/platform/src/android/presenter.rs`
- Modify: `engine/crates/platform/src/linux/presenter.rs`
- Modify: `engine/crates/platform/src/windows/presenter.rs`

- [ ] **Step 1: Add failing platform-pair tests**

Prove:

- Android identity is stable within the process;
- Linux offscreen, X11, and Wayland are distinct domains;
- X11 identities differ for different render connections;
- Wayland identities differ for different `wl_display` pointers;
- Windows HWND platforms share the pinned process ANGLE domain;
- provider and factory identities are exact matches for every constructor.

Run:

```bash
cd engine
cargo test -p migo-platform platform_identity --lib --locked --offline
```

Expected: FAIL until presenters provide identities.

- [x] **Step 2: Implement presenter identities**

Use private marker types per native domain and these instance tokens:

- Android system EGL: process token `0`;
- Linux offscreen EGL: process token `0`;
- Linux X11: the render connection identity;
- Linux Wayland: the `wl_display` identity;
- Windows ANGLE: process/device token `0` for the currently fixed ANGLE
  backend.

Provider and factory compute the same identity independently. A mixed
X11/Wayland pair or mismatched display must fail `GraphicsPlatform::try_new`.

- [ ] **Step 3: Verify presenters**

```bash
cd engine
cargo test -p migo-platform platform_identity --lib --locked --offline
cargo check -p migo-platform --all-targets --locked --offline
```

Expected: host-testable platform tests PASS. Android and Windows native target
compilation remains part of Phase C.

## Task 3: Reject Incompatible C ABI Reattachment Before Publication

**Files:**
- Modify: `engine/crates/capi/src/lib.rs`
- Modify: `engine/crates/capi/src/surface.rs`
- Modify: `engine/crates/capi/src/test_support.rs`
- Modify: `include/migo/surface.h`
- Modify: `include/migo/README.md`
- Test: `scripts/test-surface-attachment-contract.sh`

- [x] **Step 1: Add failing Session compatibility tests**

Add target-independent tests around the exact comparison helper used by attach:

- the first platform identity is accepted and committed with its Host;
- an equal identity is accepted;
- a different backend, domain, or instance returns
  `MIGO_ERROR_INVALID_STATE`;
- rejection occurs before `lease_surface_tracked` and command enqueue;
- rejection leaves the Session's existing identity, Host, generation, ingress,
  and attachment state unchanged and retryable.

Add Linux platform tests for X11-to-Wayland and different-display rejection.
Run:

```bash
cd engine
cargo test -p migo-capi platform_identity --lib --locked --offline
```

Expected: FAIL because `SessionState` has no platform identity and existing
reattach ignores the candidate `GraphicsPlatform`.

- [x] **Step 2: Enforce the commit invariant**

Store `Option<PlatformIdentity>` beside the owned Host. During attach:

1. build and validate the candidate platform;
2. compare it with the stored identity for an existing Host;
3. on mismatch, roll back the internal transition and return
   `MIGO_ERROR_INVALID_STATE`;
4. only after equality may code lease a Surface or enqueue `UpdateSurface`;
5. commit the identity and Host owner together on cold start.

Treat `Host` without identity or identity without `Host` as
`MIGO_ERROR_INTERNAL`; never infer compatibility from `HostId`, native window
kind alone, or a render-thread failure.

- [ ] **Step 3: Document and verify observable behavior**

Document supported same-domain replacement and the new synchronous error for
backend/display replacement without changing ABI layouts or versions. Run:

```bash
cd engine
cargo test -p migo-capi platform_identity --lib --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh --core
bash scripts/test-surface-attachment-contract.sh
```

Expected: PASS.

## Task 4: Close A2

- [ ] **Step 1: Run broad checks**

```bash
cd engine
cargo fmt --all -- --check
cargo test -p migo-graphics --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cargo test -p migo-capi --lib --locked --offline
cargo check -p migo-graphics -p migo-platform -p migo-capi --all-targets \
  --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh
bash scripts/test-surface-attachment-contract.sh
```

- [ ] **Step 2: Inspect the invariant**

```bash
rg -n "PlatformIdentity|platform_identity" engine include
rg -n "lease_surface_tracked|UpdateSurface" engine/crates/capi/src/surface.rs
```

Expected: every candidate comparison precedes lease/enqueue, and every shipping
presenter supplies exact provider/factory identity.

- [ ] **Step 3: Update ledgers and commit**

Mark A2 complete only after all host-testable commands pass. Commit verified
implementation and evidence locally; do not push.
