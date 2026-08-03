# Owned Host Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every spawned Migo Host thread explicitly owned and guarantee that Engine/native-host destruction joins it before native display resources or the Migo library may be released.

**Architecture:** Core returns an owning `HostThread` instead of a detached integer ID. The owner requests shutdown through the existing queue-independent `SurfaceControl` path and consumes the thread handle to join. C ABI Sessions transfer a stopped Host into an Engine-owned retirement set so callback-reentrant Session destruction never self-joins; Engine destruction drains that set outside locks. Android JNI and the desktop player retain the same owning handle and synchronously join during their terminal shutdown path.

**Tech Stack:** Rust 2024, `std::thread::JoinHandle`, crossbeam channels, C ABI, JNI, Cargo unit tests, C header contract tests.

---

## Task 1: Specify The Observable Lifecycle Contract

**Files:**
- Modify: `include/migo/session.h`
- Modify: `include/migo/README.md`
- Test: `tests/c_abi/core_contract.c`
- Test: `tests/c_abi/core_contract.cc`
- Test: `scripts/test-c-abi-surface-candidate.sh`

- [x] **Step 1: Add a failing contract assertion**

Extend the C and C++ contract fixtures so their lifecycle helper path documents
and compiles the ordering:

```c
migo_session_destroy(session);
migo_engine_destroy(engine);
/* Native display/window teardown and library unload are legal only here. */
```

Add a contract-script assertion for explicit header language that successful
Engine destruction is a thread-completion barrier. Run:

```bash
bash scripts/test-c-abi-surface-candidate.sh --core
```

Expected: FAIL because `include/migo/session.h` does not yet promise the
completion barrier.

- [x] **Step 2: Update the public contract**

State in `include/migo/session.h` and `include/migo/README.md` that:

- successful Session destruction closes/cancels public work and transfers any
  exiting Host to its Engine;
- successful Engine destruction joins all Migo-owned workers;
- native display/window teardown and library unload are safe only after Engine
  destruction returns;
- callback-reentrant Session destruction remains non-blocking with respect to
  its current callback frame.

Do not change struct layouts, exported names, or ABI version.

- [x] **Step 3: Verify the contract**

Run:

```bash
bash scripts/test-c-abi-surface-candidate.sh --core
```

Expected: PASS for C11, C++17, ILP32 layout, and public export declarations.

- [x] **Step 4: Commit**

```bash
git add include/migo/session.h include/migo/README.md \
  tests/c_abi/core_contract.c tests/c_abi/core_contract.cc \
  scripts/test-c-abi-surface-candidate.sh
git commit -m "docs: define engine thread completion barrier"
```

## Task 2: Introduce An Owning Core Host Handle

**Files:**
- Modify: `engine/crates/core/src/runtime/thread.rs`
- Modify: `engine/crates/core/src/runtime/mod.rs`
- Modify: `engine/crates/core/src/lib.rs`
- Test: `engine/crates/core/src/runtime/thread.rs`

- [x] **Step 1: Write failing ownership tests**

Add unit tests using named threads and a drop sentinel:

1. `join_waits_for_named_host_and_observes_sentinel_drop` starts a named test
   thread, holds a sentinel in that thread, releases it through a channel, joins
   through the owning handle, and proves the sentinel was dropped before join
   returned.
2. `failed_start_joins_the_spawned_thread` drives the startup-failure helper and
   proves no named thread remains after the constructor reports failure.
3. `host_thread_id_is_stable` proves command routing continues to use the exact
   allocated `HostId`.

Run:

```bash
cd engine
cargo test -p migo-core runtime::thread::tests --lib --locked --offline
```

Expected: FAIL because no owning handle or join API exists.

- [ ] **Step 2: Implement `HostThread`**

In `engine/crates/core/src/runtime/thread.rs`, add an owning type equivalent to:

```rust
pub struct HostThread {
    host_id: HostId,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HostThread {
    pub fn id(&self) -> HostId;
    pub fn request_shutdown(&self) -> Result<(), String>;
    pub fn join(self) -> EngineResult<()>;
    pub fn shutdown_and_join(self) -> EngineResult<()>;
}
```

Implementation requirements:

- `spawn_host_thread` returns `EngineResult<HostThread>`.
- `SpawnedSurfaceHost` owns `HostThread` plus its initial
  `SurfaceResourceLease`.
- Keep the `JoinHandle` from `Builder::spawn`.
- If Host initialization fails before `ready_rx` succeeds, join the spawned
  thread before returning the construction error.
- Joining the current thread is rejected as an internal ownership error before
  calling `JoinHandle::join`.
- A thread panic becomes `EngineError::Internal` with the Host ID in its detail.
- Normal shutdown still sets `SurfaceControl::shutdown()` before the best-effort
  command nudge.
- Do not add sleeps, polling joins, detached reaper threads, or time-based
  success assumptions.

Re-export `HostThread` from `runtime/mod.rs` and `core/lib.rs`. Update the
crate-level example to retain the owner and call `shutdown_and_join`.

- [ ] **Step 3: Run focused tests and static checks**

```bash
cd engine
cargo test -p migo-core runtime::thread::tests --lib --locked --offline
cargo check -p migo-core --locked --offline
```

Expected: PASS with the new behavioral tests executing.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/core/src/runtime/thread.rs \
  engine/crates/core/src/runtime/mod.rs engine/crates/core/src/lib.rs
git commit -m "core: retain ownership of host threads"
```

## Task 3: Make C ABI Engine Destruction The Join Barrier

**Files:**
- Modify: `engine/crates/capi/src/lib.rs`
- Modify: `engine/crates/capi/src/surface.rs`
- Modify: `engine/crates/capi/src/test_support.rs`
- Test: `engine/crates/capi/src/lib.rs`
- Test: `engine/crates/capi/src/surface.rs`
- Test: `engine/crates/capi/src/callbacks.rs`

- [x] **Step 1: Write failing C ABI lifecycle tests**

Add behavioral tests that use a named worker and sentinel through the same
Engine retirement abstraction used by production:

1. successful Session destruction requests shutdown and transfers the owned Host
   without waiting on the current callback frame;
2. Engine destruction blocks until the transferred worker exits and its
   sentinel is dropped;
3. Engine destruction never holds `live_sessions`, Session state, or retirement
   locks while joining;
4. rejected Session destruction leaves Host ownership unchanged and retryable;
5. attach rollback retires and joins/transfers a newly spawned Host exactly
   once;
6. the existing
   `a_callback_can_destroy_its_session_and_the_task_pins_allocation_until_return`
   test remains passing.

Extract a small private `OwnedWorker`/retirement helper only if it lets the tests
exercise real join behavior without creating EGL/V8. Production Host retirement
must store `migo_core::HostThread`, not a mock or a second lifecycle mechanism.

Run:

```bash
cd engine
cargo test -p migo-capi engine_ --lib --locked --offline
cargo test -p migo-capi callback_can_destroy --lib --locked --offline
```

Expected: FAIL because Engine has no retirement set and Session stores only an
integer Host ID.

- [ ] **Step 2: Add Engine-owned retirement**

In `engine/crates/capi/src/lib.rs`:

- change `SessionState.host` to `Option<HostThread>`;
- add `retired_hosts: Mutex<Vec<HostThread>>` to `EngineInner`;
- add private methods that request shutdown before inserting a Host and that
  drain/join outside the mutex;
- retain the existing successful logical-destruction boundary: validate all
  guards, close callbacks, move Host ownership, decrement `live_sessions`,
  release locks, wait for permitted callback drain, request shutdown, and
  transfer ownership;
- make `migo_engine_destroy` reject live Sessions, drain all retired Hosts, join
  them without any Migo lock held, and consume the Engine only after all joins
  succeed;
- preserve retry semantics on every pre-consumption failure.

Do not join inside `migo_session_destroy`; that would violate the documented
callback-reentrant path.

In `engine/crates/capi/src/surface.rs`:

- route commands through `HostThread::id()`;
- install the owning handle only at the attachment commit boundary;
- on every cold-start rollback, request shutdown and either join immediately or
  transfer to `EngineInner` exactly once;
- never recover ownership by looking up a `HostId` in a global map.

Update `test_support.rs` construction for the retirement set.

- [ ] **Step 3: Verify C ABI lifecycle behavior**

```bash
cd engine
cargo test -p migo-capi --lib --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh
bash scripts/test-surface-attachment-contract.sh
```

Expected: all tests PASS, including callback-reentrant destruction and surface
rollback coverage.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/capi/src/lib.rs engine/crates/capi/src/surface.rs \
  engine/crates/capi/src/test_support.rs engine/crates/capi/src/callbacks.rs
git commit -m "capi: join retired hosts at engine destruction"
```

## Task 4: Give Android JNI Explicit Host Ownership

**Files:**
- Modify: `engine/crates/platform/src/android/jni/inbound.rs`
- Test: `engine/crates/platform/src/android/jni/inbound.rs`
- Test: `scripts/test-android-sdk-contract.sh`

- [x] **Step 1: Write failing Android ownership tests**

Add target-independent unit tests around a private ownership registry that prove:

- insertion returns the exact `HostId`;
- duplicate insertion fails closed without replacing a live owner;
- terminal removal transfers ownership exactly once;
- unknown/already-stopped IDs are idempotent at the JNI boundary;
- shutdown removes hot ingress before joining and no command can be routed after
  terminal removal.

Run:

```bash
cd engine
cargo test -p migo-platform android::jni::inbound --lib --locked --offline
```

Expected: FAIL because JNI discards `HostThread` and stores only the returned ID.

- [ ] **Step 2: Retain and join Android Hosts**

Add a process-owned, mutex-protected `HashMap<HostId, HostThread>` in the Android
JNI layer. On `init`, insert the owner only after successful construction and
return `owner.id()` to Java. On `shutdown`, invalidate hot ingress, remove the
owner, request queue-independent shutdown, and join before returning.

The registry is ownership, not command routing:

- existing high-frequency command paths continue to use direct `HostIngress`;
- normal commands continue to route by `HostId`;
- the mutex is never held during join;
- no sleep, retry loop, leaked owner, detached helper thread, or process-exit
  cleanup is accepted.

- [ ] **Step 3: Verify Android code paths**

```bash
cd engine
cargo test -p migo-platform android::jni::inbound --lib --locked --offline
cargo check -p migo-platform --locked --offline
cd ..
bash scripts/test-android-sdk-contract.sh
```

Expected: unit tests and host-testable Android contract checks PASS. A real
Android target build remains part of Phase C.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/platform/src/android/jni/inbound.rs \
  scripts/test-android-sdk-contract.sh
git commit -m "android: own and join native host threads"
```

## Task 5: Make The Desktop Player Join Before Native Teardown

**Files:**
- Modify: `engine/tools/player/src/main.rs`
- Modify: `engine/tools/player/src/win32_window.rs`
- Test: `engine/tools/player/src/main.rs`

- [x] **Step 1: Add a failing teardown-order test**

Extract the terminal sequence into a helper whose injected test resources record
these events:

```text
shutdown requested
host thread joined
window/display resource dropped
```

Assert exact ordering and that a join failure prevents success from being
reported. Run:

```bash
cd engine
cargo test -p migo-player teardown --locked --offline
```

Expected: FAIL because the player discards the JoinHandle and only signals
shutdown.

- [ ] **Step 2: Retain the owner through the player loop**

Keep `HostThread` in `main`, pass only `owner.id()` to routing helpers, and call
`shutdown_and_join()` before dropping Win32/X11/Wayland window/display state.
Preserve capture-on-close behavior and return join failures from the process.

- [ ] **Step 3: Verify player behavior**

```bash
cd engine
cargo test -p migo-player teardown --locked --offline
cargo check -p migo-player --locked --offline
```

Expected: PASS and no detached `Migo-Main-*` thread path remains.

- [ ] **Step 4: Commit**

```bash
git add engine/tools/player/src/main.rs engine/tools/player/src/win32_window.rs
git commit -m "player: join host before native teardown"
```

## Task 6: Close The Lifecycle Phase With Broad Verification

**Files:**
- Modify: `docs/superpowers/plans/2026-07-29-a1-owned-host-lifecycle.md`
- Modify: `docs/superpowers/plans/2026-07-29-three-platform-delivery.md`

- [ ] **Step 1: Run formatting and focused verification**

```bash
cd engine
cargo fmt --all -- --check
cargo test -p migo-core --lib --locked --offline
cargo test -p migo-capi --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cargo test -p migo-player --locked --offline
cargo check -p migo-capi -p migo-platform -p migo-player --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh
bash scripts/test-surface-attachment-contract.sh
bash scripts/test-platform-services-capability-contract.sh
```

Expected: every command exits 0; test output shows the new lifecycle behavioral
tests executed rather than filtered to zero tests.

- [ ] **Step 2: Inspect the implementation**

```bash
rg -n "spawn_host_thread\\(|spawn_host_thread_tracked\\(" engine
rg -n "shutdown_host\\(" engine
rg -n "JoinHandle|HostThread|retired_hosts" engine
```

Expected:

- every production `spawn_host_thread*` result has an owning `HostThread`;
- signal-only `shutdown_host` remains only where ownership is retained
  elsewhere or inside `HostThread`;
- no production path discards a `JoinHandle`;
- no lifecycle success path uses sleeps, `|| true`, or process-exit cleanup.

- [ ] **Step 3: Update plan ledgers**

Mark every completed checkbox in this plan and A1 in the umbrella plan. Record
the exact verification commands and any platform-native evidence deferred to
Phase C; do not mark A1 complete if a host-testable command fails.

- [ ] **Step 4: Commit the verified A1 state**

```bash
git add -f docs/superpowers/plans/2026-07-29-a1-owned-host-lifecycle.md \
  docs/superpowers/plans/2026-07-29-three-platform-delivery.md
git commit -m "docs: record verified host lifecycle implementation"
```
