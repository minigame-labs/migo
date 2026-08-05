# Runtime Restart Generation Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `HostCommand::Restart` incapable of delivering a retired runtime's asynchronous result, resource event, or stream event into the replacement JavaScript isolate, while preserving Android's public callback interfaces.

**Architecture:** A `CallbackIdAllocator` owned by `Host` issues positive `i32` IDs once for the Host lifetime and is shared through every `HostOpState`, including Workers. ID-bearing operations use exact lookup with no FIFO fallback, and every runtime-owned callback also carries the generation that created it so `Host` rejects retired work before invoking JavaScript. Direct Host ingress stamps the current generation at enqueue. Resource creation carries the caller's expected generation through the platform boundary and is admitted only by an exact generation match. Restart first establishes a synchronous platform revocation fence, joins retired Workers, and performs ordered audio/render resets, then constructs and evaluates an unpublished candidate isolate; only a completely initialized candidate is committed.

**Tech Stack:** Rust 2024, Deno Core/V8 ops and extension ESM, atomic integers, Tokio/crossbeam channels, JNI, Java `ConcurrentHashMap`, Android managers, Gradle/JUnit, Cargo tests, V8 startup snapshots.

---

## Execution Rules

- The design authority is Section 6.6 of
  `docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md`.
- Work in `.worktrees/three-platform-delivery`; do not combine these changes
  with unrelated delivery work already present in the worktree.
- Before each task, require an empty task-owned index. If the worktree contains
  earlier changes, finish and verify their checkpoint or create an isolated
  worktree; never use a broad `git add` against a mixed index. Immediately
  before each commit, compare `git diff --cached --name-only` with that task's
  declared file list and inspect `git diff --cached --check`.
- Run every red test before its implementation step. A test that unexpectedly
  passes must be corrected so it observes the missing behavior.
- Every filtered Cargo invocation uses
  `scripts/run-cargo-lib-test-filter.sh`, which first lists matching tests and
  fails when the match count is zero. A raw filtered `cargo test` exit status is
  never accepted as evidence.
- Logical revocation happens before physical Android teardown. Exclusive and
  power-sensitive teardown may hop to the main or manager thread only through
  an owned completion barrier that the fence awaits; the new generation cannot
  acquire those resources until it completes. Non-exclusive work may finish
  later only when its generation token prevents callback delivery.
- Do not change `AdHandler`, `AdEventSink`, `PermissionHandler`,
  `PermissionSink`, `AuthHandler`, or `SubpackageHandler` public method
  signatures. Java methods under `com.migo.runtime.internal` and JNI
  signatures are internal and may carry correlation data.
- IDs and generations are never reset, recycled, or wrapped. Exhaustion is a
  permanent error for the remaining Host lifetime.
- A cleanup timeout is diagnostic only and never releases ownership. After a
  restart cleanup error, the Host enters a cleanup-only terminal state and may
  exit only after the platform, Worker, audio, and render completion barriers
  have all reached quiescence. Failed handles remain owned and retryable.
- Each commit below is local only. Do not push, tag, publish, or rewrite other
  local commits.

## File And Ownership Map

New focused units:

- `engine/crates/shared/src/callback_id.rs`: Host-lifetime positive `i32`
  allocation and exhaustion semantics.
- `engine/crates/core/src/runtime/restart_boundary.rs`: checked runtime
  generation transition and restart phase/commit state used by `Host` tests.
- `engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs`: behavioral
  JavaScript registry, stale-result, and exhaustion tests.
- `platforms/android/library/src/main/java/com/migo/runtime/internal/RuntimeGenerationBoundary.java`:
  per-session generation tokens plus active-ad and pending-permission sets.
- `platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeGenerationBoundaryTest.java`:
  Java fence and stale-token tests.
- `scripts/test-runtime-restart-generation-contract.sh`: exhaustive static
  audit that rejects reintroduced local host callback counters and ID-less
  result fallback. It supplements, but never replaces, behavioral tests.
- `scripts/run-cargo-lib-test-filter.sh`: fail-closed wrapper that proves a
  focused Cargo filter selects at least one named library test before running
  it.

Existing ownership boundaries:

- `engine/crates/core/src/runtime/host.rs`: owns the allocator and authoritative
  `RestartBoundary`; the registry has only a `RuntimeGenerationReader` clone
  for stamping direct Host ingress. Host sequences platform, audio, render, V8
  drop, candidate load, and publication.
- `engine/crates/shared/src/op_state/mod.rs`: carries an `Arc` clone of the
  Host allocator and the immutable generation of that runtime.
- `engine/crates/runtime-v8/src/base/mod.rs` and `base/02_async.js`: expose the
  allocation op and the only JS validation/allocation helpers.
- `engine/crates/core/src/services/platform.rs`: defines the cross-platform
  restart-fence capability.
- `engine/crates/platform/src/android/platform.rs` and
  `engine/crates/platform/src/android/jni/outbound.rs`: invoke the synchronous
  Java fence.
- `engine/crates/shared/src/protocol/audio_cmd.rs`,
  `engine/crates/core/src/services/audio.rs`, and
  `engine/crates/audio/src/audio_thread.rs`: implement the ordered audio reset.
- `engine/crates/shared/src/protocol/render_cmd.rs`,
  `engine/crates/core/src/services/render.rs`,
  `engine/crates/graphics/src/render_thread.rs`, and
  `engine/crates/graphics/src/canvas/manager/mod.rs`: implement the ordered
  render reset while preserving the compatible native Surface lease.

## Exhaustive Callback And Counter Audit

The following local counters cross the runtime boundary and must migrate to
`allocateHostCallbackId()`:

| Current owner | Operations/resources | Required disposition |
|---|---|---|
| `base/02_async.js` `_nextId` | location, fuzzy location, scan, compress image, choose image, choose message file, share, Bluetooth settings, authorize, open settings, navigate | Delete counter; allocate once immediately before pending-map insertion and platform dispatch |
| `system/13_login.js` `_nextRequestId` | login, checkSession, getUserInfo, getPhoneNumber, user-info authorization flow | Delete counter; use shared allocator and exact ID parsing |
| `payment/01_payment.js` `_nextRequestId` | Midas and Midas game-item payments | Delete counter and both oldest-pending fallbacks |
| `base/04_subpackage.js` `_nextRequestId` | load progress and terminal result | Delete counter; progress and result share one Host ID |
| `ad/01_ad.js` `_nextAdId` | banner, rewarded, interstitial, custom, and grid ads | Delete counter/wrap loop; constructor fails before registry/JNI work on exhaustion |
| `media/01_camera.js` `_nextCameraId` | camera events and frame data | Delete counter; retain positive ID as `u32` only after checked conversion |
| `media/04_video.js` `_nextVideoId` | video state/events | Delete counter; Android `VideoManager.create` must use the supplied ID |
| `audio/02_inner_audio_context.js` `nextInnerAudioId` | inner-audio event routing | Delete counter; reset barrier removes retired players |
| `ui/01_interaction.js` FIFO arrays | modal and action-sheet results | Replace each FIFO with a map keyed by a Host ID carried through internal Android/JNI callbacks |
| `system/02_authorize.js` `_pendingAuthSetting` FIFO | application authorization settings result | Replace FIFO with a Host-ID map and carry the ID through internal Android/JNI callbacks |

The callback-hook audit is closed over these exact 27 bridge entry points.
These 25 must carry an exact Host-allocated ID, perform exact lookup, and carry
the positive generation captured when the request or resource was created:

```text
_internalOnActionSheetResult, _internalOnAdEvent, _internalOnAuthorizeResult,
_internalOnCameraEvent, _internalOnCameraFrameData,
_internalOnCheckSessionResult, _internalOnChooseImageResult,
_internalOnChooseMessageFileResult, _internalOnCompressImageResult,
_internalOnFuzzyLocationResult, _internalOnGetPhoneNumberResult,
_internalOnGetUserInfoResult, _internalOnLocationResult,
_internalOnLoginResult, _internalOnMidasPaymentGameItemResult,
_internalOnMidasPaymentResult, _internalOnModalResult,
_internalOnNavigateToMiniProgramResult,
_internalOnOpenAppAuthorizeSettingFinished,
_internalOnOpenBluetoothSettingResult, _internalOnOpenSettingResult,
_internalOnScanCodeResult, _internalOnShareAppMessageResult,
_internalOnSubpackageProgress, _internalOnSubpackageResult
```

The remaining two, `_internalOnRecorderEvent` and
`_internalOnRecorderFrameData`, carry a runtime generation without a request
ID. Video and inner-audio use dedicated binding dispatch rather than
`_internalOn*` JSON hooks; they carry both their exact binding ID and captured
generation. Their stale-event tests remain mandatory in Tasks 5 and 11.

These callback sources have no natural request ID and must capture a runtime
generation token at manager/work creation. They test it immediately before
calling `NativeMethods` as an early-drop optimization, and carry its positive
`long` value through `NativeBridge`, JNI ingress, and `HostCommand` for the
authoritative Host-generation comparison:

- Recorder events and frame data from `audio/05_recorder_manager.js` /
  `AudioRecorderManager`.
- Device motion, gyroscope, compass, accelerometer, and orientation streams
  from `DeviceSensorManager`.
- Network status from `NetworkMonitor`.
- Bluetooth adapter/device discovery, beacon, BLE connection,
  characteristic-value, and MTU events from `BluetoothManager`.
- Keyboard input, confirm, complete, and height streams from `KeyboardManager`.
- User-capture-screen events from `ScreenCaptureObserver`.
- Dialog and `ResultProxyActivity` closures that may run after their visible UI
  has been dismissed.

Long-running Android auth and subpackage handler completions are ID-bearing,
but their public callback interfaces cannot be cancelled. Every callback
object captures a generation token and uses it for early drop before calling
`NativeMethods`; its result JSON still echoes the original Host ID. JNI queues
both values, `Host` rejects a generation mismatch before invoking any
JavaScript hook, and only then may the current runtime perform an exact
pending-map lookup.

ID-bearing resources still need physical cleanup and active-set checks:

- Ads: active ad IDs are added before `AdHandler.createAd`, removed before
  explicit destroy, cleared by the restart fence, and checked by every
  `SessionAdEventSink.emit` path.
- Permissions: each `permissionRequest` adds a pending ID and receives a
  generation-specific `SessionPermissionSink`; resolve/fail consumes the ID,
  while restart clears pending IDs. `setScope` is accepted only from a current
  token. Previously committed standing scope decisions remain in Rust.
- Location: each fresh `LocationListener` and timeout is tracked by
  `(sessionId, requestId)` and cancelled by the fence.
- Camera, video, and inner audio: IDs prevent stale lookup; manager/player reset
  stops resource use and event production.

Camera, recorder, location, sensor, Bluetooth discovery, beacon, BLE/GATT, and
their worker threads are exclusive or power-sensitive. Their stop/unregister/
close operations participate in the Android fence completion barrier. Every
resource-creating op passes `HostOpState.runtime_generation` through
`PlatformServices`; Android calls `requireActive(sessionId,
expectedGeneration)` under the same per-session lock that advances the
generation. Thus an old Worker cannot acquire under the replacement
generation, even after it observed an earlier active state. The restart also
stops and joins all retired Workers before candidate publication. Image
compression, auth, and subpackage work may finish after the fence because they
retain no exclusive resource; their captured token drops the completion.

The following counters intentionally remain outside this allocator:

- `web/02_timers.js` timer IDs, Deno resource-table RIDs, fetch/socket RIDs, and
  file/download task IDs are isolate-local and cannot enter a replacement
  isolate through a platform callback.
- `rendering/webgl/webgl.rs::IdAllocator` remains per runtime because WebGL
  object allocation is high-volume and has no direct platform callback. The
  synchronous render reset, not a V8 op per object, prevents native collision.
- `audio/01_audio_context.js` context/node IDs and
  `audio/04_media_audio_player.js` player IDs remain isolate-local because no
  platform callback routes by them. The ordered audio reset proves every old
  registry and decode destination is gone before those counters can restart;
  putting high-volume audio-node creation through a V8 op is unnecessary.
- `web/canvas.rs::NEXT_JS_OFFSCREEN_CANVAS_ID` and shared image IDs already have
  process/Host-safe allocation; the render barrier owns their physical reset.
- Memory pressure, thermal status, Surface state, `OnShow`/`OnHide`, audio
  interruption, and render context-loss reconciliation describe current
  Session/OS state rather than completion of old runtime work. They remain
  Host-owned current-state events.

## Task 1: Add The Fail-Closed Host Callback Allocator

**Files:**
- Create: `scripts/run-cargo-lib-test-filter.sh`
- Create: `engine/crates/shared/src/callback_id.rs`
- Modify: `engine/crates/shared/src/lib.rs`
- Test: `engine/crates/shared/src/callback_id.rs`

- [ ] **Step 1: Add the fail-closed filtered-test runner**

Create `scripts/run-cargo-lib-test-filter.sh`. It accepts exactly a Cargo
package and test-name filter, runs `cargo test -p <package> <filter> --lib
--locked --offline -- --list`, parses the listed test identities, and fails
unless at least one test matches. It then runs the same filtered test command.
Its `--self-test` mode feeds the parser zero, one, and multiple test identities
and proves that only the zero-match case fails. Run the self-test before using
the wrapper anywhere in this plan.

```bash
bash scripts/run-cargo-lib-test-filter.sh --self-test
```

- [ ] **Step 2: Write allocator boundary tests**

Add tests named
`starts_at_one_and_never_reuses_ids`,
`arc_clones_allocate_one_global_sequence`,
`threads_never_duplicate_an_id`, and
`maximum_is_issued_once_then_exhaustion_is_permanent`. The last test uses a
test-only constructor initialized to `i32::MAX as u32 - 1` and asserts:

```rust
assert_eq!(ids.allocate().unwrap(), i32::MAX);
assert_eq!(ids.allocate(), Err(CallbackIdExhausted));
assert_eq!(ids.allocate(), Err(CallbackIdExhausted));
```

- [ ] **Step 3: Run the tests and verify the red state**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-shared callback_id
```

Expected: FAIL because `shared::callback_id` does not exist.

- [ ] **Step 4: Implement the allocator without wrap or recycling**

Implement this public shape and export the module from `shared/src/lib.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackIdExhausted;

#[derive(Debug, Default)]
pub struct CallbackIdAllocator {
    last_issued: std::sync::atomic::AtomicU32,
}

impl CallbackIdAllocator {
    pub fn allocate(&self) -> Result<i32, CallbackIdExhausted> {
        use std::sync::atomic::Ordering::Relaxed;
        let previous = self.last_issued.fetch_update(Relaxed, Relaxed, |last| {
            (last < i32::MAX as u32).then_some(last + 1)
        }).map_err(|_| CallbackIdExhausted)?;
        Ok((previous + 1) as i32)
    }
}
```

Implement `Display` and `Error` with the stable message
`"host callback id space exhausted"`. Keep the near-maximum constructor under
`#[cfg(test)]`; do not expose reset, release, or free methods.

- [ ] **Step 5: Run shared tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-shared callback_id
cargo check -p migo-shared --locked --offline
```

Expected: PASS; the concurrency test observes exactly its requested count of
distinct IDs, all in `1..=i32::MAX`.

- [ ] **Step 6: Commit the allocator and test runner**

```bash
git add scripts/run-cargo-lib-test-filter.sh \
  engine/crates/shared/src/callback_id.rs engine/crates/shared/src/lib.rs
git commit -m "runtime: add host callback id allocator"
```

## Task 2: Carry One Allocator And Generation Boundary Through The Host

**Files:**
- Modify: `engine/crates/shared/src/op_state/mod.rs`
- Modify: `engine/crates/core/src/runtime/mod.rs`
- Modify: `engine/crates/core/src/runtime/host.rs`
- Modify: `engine/crates/core/src/runtime/registry.rs`
- Modify: `engine/crates/core/src/runtime/thread.rs`
- Modify: `engine/crates/runtime-v8/src/host_runtime.rs`
- Modify: `engine/crates/runtime-v8/src/worker/mod.rs`
- Modify: `engine/crates/runtime-v8/src/permission.rs`
- Modify: `engine/crates/runtime-v8/src/snapshot.rs`
- Modify: `engine/crates/runtime-v8/src/rendering/webgl/webgl.rs`
- Modify: every `HostOpState` test literal under `engine/crates/runtime-v8/src/tests/`
- Test: `engine/crates/core/src/runtime/restart_boundary.rs`
- Test: `engine/crates/runtime-v8/src/worker/mod.rs`

- [ ] **Step 1: Add failing ownership and Worker-sharing tests**

Declare `mod restart_boundary;` in `runtime/mod.rs`. Create
`runtime/restart_boundary.rs` with `RestartBoundary` and the cloneable,
read-only `RuntimeGenerationReader`; both share one private `Arc<AtomicI64>`
initialized to `1`. In the same test module, construct one
`Arc<CallbackIdAllocator>`. Assert
that two simulated runtime states and one Worker state hold `Arc::ptr_eq`
allocator clones and allocate `1`, `2`, and `3` across their interleaving.
Assert `RestartBoundary::current()` and its reader both return `1`, and that the
reader exposes no mutation API. Assert candidate generation `2` does not
mutate current, exact commit makes both readers observe `2`, a stale commit is
rejected, and a test-only boundary at `i64::MAX` fails candidate generation
without mutation. Add a Worker unit test named
`worker_and_parent_share_host_callback_id_space` that performs the same check
through the Worker-state construction helper.

- [ ] **Step 2: Run the focused tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 worker_and_parent_share_host_callback_id_space
```

Expected: FAIL because neither `Host` nor `HostOpState` carries the allocator.

- [ ] **Step 3: Add the fields and preserve them across restart**

Add to `HostOpState`:

```rust
pub callback_ids: Arc<crate::callback_id::CallbackIdAllocator>,
pub runtime_generation: i64,
```

Add to `Host`:

```rust
callback_ids: Arc<CallbackIdAllocator>,
restart_boundary: RestartBoundary,
```

Construct the allocator once in `Host::new`. Construct `RestartBoundary` once
in `spawn_host_thread_inner`, before sender registration, pass its reader to
`register_sender`, and pass the boundary itself to `Host::new`. `RegisteredHost`
and `HostIngress` retain reader clones; expose
`HostIngress::runtime_generation()` as an acquire load with no setter. Copy
`RestartBoundary::current()` into the initial `HostOpState`; until Task 10
introduces an unpublished candidate, the existing restart construction also
copies current. In `WorkerManager::create_worker`, clone
`host_state.callback_ids` and copy `host_state.runtime_generation`; never
construct a Worker-local allocator or generation.

Implement this complete production surface now so later tasks cannot invent a
second generation authority:

```rust
pub(crate) struct RestartBoundary {
    current: Arc<AtomicI64>,
}

#[derive(Clone)]
pub(crate) struct RuntimeGenerationReader {
    current: Arc<AtomicI64>,
}

impl RestartBoundary {
    pub(crate) fn new() -> Self;
    pub(crate) fn current(&self) -> i64;
    pub(crate) fn reader(&self) -> RuntimeGenerationReader;
    pub(crate) fn candidate_generation(&self) -> EngineResult<i64>;
    pub(crate) fn commit(&self, retired: i64, candidate: i64) -> EngineResult<()>;
}

impl RuntimeGenerationReader {
    pub(crate) fn current(&self) -> i64;
}
```

Both `current` methods use `Acquire`. `candidate_generation()` uses
`checked_add(1)` and returns `ErrorCode::InvalidOperation` with detail
`"runtime generation exhausted"` without mutation at `i64::MAX`.
`commit(retired, candidate)` requires `candidate == retired + 1` and uses
`compare_exchange(retired, candidate, AcqRel, Acquire)`. Keep a near-maximum
constructor under `#[cfg(test)]`; expose no reset, direct store, or rollback.

- [ ] **Step 4: Update every explicit `HostOpState` literal**

Add an isolated allocator and generation `1` to all literals in these exact
files:

```text
engine/crates/runtime-v8/src/permission.rs
engine/crates/runtime-v8/src/rendering/webgl/webgl.rs
engine/crates/runtime-v8/src/snapshot.rs
engine/crates/runtime-v8/src/tests/ad_reward_integrity.rs
engine/crates/runtime-v8/src/tests/binary_helper.rs
engine/crates/runtime-v8/src/tests/canvas_follows_surface.rs
engine/crates/runtime-v8/src/tests/global_surface.rs
engine/crates/runtime-v8/src/tests/host_bridge_dispatch.rs
engine/crates/runtime-v8/src/tests/permission_reporting.rs
engine/crates/runtime-v8/src/tests/permission_revocation.rs
engine/crates/runtime-v8/src/tests/published_namespace_isolation.rs
engine/crates/runtime-v8/src/tests/storage_isolation.rs
engine/crates/runtime-v8/src/tests/timers.rs
engine/crates/runtime-v8/src/tests/v8_limits.rs
engine/crates/runtime-v8/src/worker/mod.rs
```

Verify completeness with:

```bash
rg -n 'HostOpState \{' engine/crates --glob '*.rs'
```

Every printed construction must set both fields; do not add a global default
allocator that would accidentally merge independent test Hosts.

- [ ] **Step 5: Run wiring tests and checks**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 worker_and_parent_share_host_callback_id_space
cargo check -p migo-core -p migo-runtime-v8 --locked --offline
```

Expected: PASS with no missing-field errors.

- [ ] **Step 6: Commit Host and Worker ownership**

```bash
git add engine/crates/shared/src/op_state/mod.rs \
  engine/crates/core/src/runtime/mod.rs \
  engine/crates/core/src/runtime/host.rs \
  engine/crates/core/src/runtime/registry.rs \
  engine/crates/core/src/runtime/thread.rs \
  engine/crates/core/src/runtime/restart_boundary.rs \
  engine/crates/runtime-v8/src/permission.rs \
  engine/crates/runtime-v8/src/rendering/webgl/webgl.rs \
  engine/crates/runtime-v8/src/snapshot.rs \
  engine/crates/runtime-v8/src/tests/ad_reward_integrity.rs \
  engine/crates/runtime-v8/src/tests/binary_helper.rs \
  engine/crates/runtime-v8/src/tests/canvas_follows_surface.rs \
  engine/crates/runtime-v8/src/tests/global_surface.rs \
  engine/crates/runtime-v8/src/tests/host_bridge_dispatch.rs \
  engine/crates/runtime-v8/src/tests/permission_reporting.rs \
  engine/crates/runtime-v8/src/tests/permission_revocation.rs \
  engine/crates/runtime-v8/src/tests/published_namespace_isolation.rs \
  engine/crates/runtime-v8/src/tests/storage_isolation.rs \
  engine/crates/runtime-v8/src/tests/timers.rs \
  engine/crates/runtime-v8/src/tests/v8_limits.rs \
  engine/crates/runtime-v8/src/worker/mod.rs
git commit -m "runtime: share callback ids across restarts and workers"
```

## Task 3: Expose Strict JavaScript Allocation And Result Parsing

**Files:**
- Modify: `engine/crates/runtime-v8/src/base/mod.rs`
- Modify: `engine/crates/runtime-v8/src/base/02_async.js`
- Create: `engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs`
- Modify: `engine/crates/runtime-v8/src/tests/mod.rs`

- [ ] **Step 1: Write failing base-helper behavior tests**

Build two `HostJsRuntime` instances sequentially with the same allocator. In
the first, create two deferred requests and record their IDs. Drop it; in the
second, create a request and assert its ID is greater than both old IDs. Inject
an old result and assert the second callback is untouched, then inject its
exact ID and assert one settlement.

Add cases that pass missing, `0`, `-1`, `1.5`, `NaN`, `Infinity`, and
`2147483648` IDs to `settle`; assert every case leaves the pending request
untouched. Add an exhaustion test asserting no pending-map entry and no host op
dispatch occurs when allocation throws.

- [ ] **Step 2: Run the focused test and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 tests::runtime_restart_boundary
```

Expected: FAIL because the base op/helper does not exist and missing IDs still
settle the oldest request.

- [ ] **Step 3: Add the fast op and JS helpers**

Register this op in `host_v8_base`:

```rust
#[op2(fast)]
fn op_alloc_host_callback_id(state: &mut OpState) -> Result<i32, JsErrorBox> {
    state.borrow::<HostOpState>()
        .callback_ids
        .allocate()
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}
```

In `02_async.js`, export one allocator and one parser:

```js
const MAX_HOST_CALLBACK_ID = 2147483647;

function parseHostCallbackId(value) {
  const id = Number(value);
  return Number.isInteger(id) && id > 0 && id <= MAX_HOST_CALLBACK_ID ? id : null;
}

function allocateHostCallbackId() {
  const id = op_alloc_host_callback_id();
  if (parseHostCallbackId(id) === null) throw new Error("invalid host callback id");
  return id;
}
```

Delete `createDeferredApi`'s `_nextId`. Allocate immediately before `pending.set`
and platform invocation. In `settle`, require `parseHostCallbackId` and exact
`pending.get(id)`; delete the `pending.values().next()` compatibility path.

- [ ] **Step 4: Run base behavior tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 tests::runtime_restart_boundary
```

Expected: PASS, including stale result, invalid ID, and no-dispatch exhaustion
cases.

- [ ] **Step 5: Commit the base correlation contract**

```bash
git add engine/crates/runtime-v8/src/base/mod.rs \
  engine/crates/runtime-v8/src/base/02_async.js \
  engine/crates/runtime-v8/src/tests/mod.rs \
  engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs
git commit -m "runtime: require exact host callback ids"
```

## Task 4: Migrate Request And Progress Registries

**Files:**
- Modify: `engine/crates/runtime-v8/src/system/13_login.js`
- Modify: `engine/crates/runtime-v8/src/payment/01_payment.js`
- Modify: `engine/crates/runtime-v8/src/base/04_subpackage.js`
- Test: `engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs`

- [ ] **Step 1: Add failing registry-family tests**

For login, payment, and subpackage APIs, capture the outgoing JSON and assert
the IDs are allocated from the same sequence as a generic deferred request.
Inject old-runtime IDs into a fresh runtime and assert no success, failure,
complete, progress, or Promise resolution callback fires. For both payment
result hooks, omit `requestId` while one request is pending and assert it stays
pending.

- [ ] **Step 2: Run the tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::request_registry
```

Expected: FAIL because the three modules still own counters and payment still
has FIFO fallback.

- [ ] **Step 3: Replace all three counters**

Import `allocateHostCallbackId` and `parseHostCallbackId` from
`ext:host_v8_base/02_async.js`. Replace every `_nextId()` and
`_nextRequestId++`. Parse every progress/result ID with the shared parser;
unknown IDs return without callbacks. Remove both payment branches that select
the first entry from `_pendingMidas` or `_pendingMidasGameItem`.

Allocation errors must reject the operation before any pending-map insertion or
host op call. Preserve each API's existing `fail` and `complete` callback shape
for ordinary platform errors.

- [ ] **Step 4: Run request-family tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::request_registry
```

Expected: PASS for login, payment, and subpackage progress/terminal paths.

- [ ] **Step 5: Commit request migration**

```bash
git add engine/crates/runtime-v8/src/system/13_login.js \
  engine/crates/runtime-v8/src/payment/01_payment.js \
  engine/crates/runtime-v8/src/base/04_subpackage.js \
  engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs
git commit -m "runtime: correlate auth payment and subpackage results"
```

## Task 5: Migrate Callback-Routed Resources And ID-Less UI Results

**Files:**
- Modify: `engine/crates/runtime-v8/src/ad/01_ad.js`
- Modify: `engine/crates/runtime-v8/src/media/01_camera.js`
- Modify: `engine/crates/runtime-v8/src/media/04_video.js`
- Modify: `engine/crates/runtime-v8/src/audio/02_inner_audio_context.js`
- Modify: `engine/crates/runtime-v8/src/ui/01_interaction.js`
- Modify: `engine/crates/runtime-v8/src/system/02_authorize.js`
- Modify: `engine/crates/runtime-v8/src/ui/mod.rs`
- Modify: `engine/crates/runtime-v8/src/system/mod.rs`
- Modify: `engine/crates/shared/src/services/interaction.rs`
- Modify: `engine/crates/shared/src/services/system_info.rs`
- Modify: `engine/crates/platform/src/android/services/mod.rs`
- Test: `engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs`

- [ ] **Step 1: Add failing stale-resource and UI tests**

Create old-generation ad, camera, video, and inner-audio objects, then create
the same categories after restart. Assert all new IDs differ. Inject old
ad/camera/video/inner-audio events and assert no new listener fires.

Issue two modal calls, two action-sheet calls, and two app-authorization-setting
calls. Deliver results out of order by ID and assert each resolves its matching
Promise. Deliver an ID-less result and assert neither pending entry settles.

- [ ] **Step 2: Run focused tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::resource_registry
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::ui_result
```

Expected: FAIL because resource counters reset and UI/settings results are FIFO.

- [ ] **Step 3: Replace resource counters**

Import and call `allocateHostCallbackId()` in the ad, camera, video,
and inner-audio construction paths. Convert to Rust `u32` only after the JS
helper has guaranteed `1..=i32::MAX`. Delete the ad wrap-and-probe loop.
Allocation exhaustion throws before registry insertion and before
`op_ad_create`, `op_camera_create`, `op_video_create`, or inner-audio command
dispatch.

- [ ] **Step 4: Replace UI and app-settings FIFO correlation**

Store pending modal, action sheet, and application-settings operations in
`Map<number, Pending>`. Include `requestId` in the existing modal/action JSON;
change `op_open_app_authorize_setting` to accept an `i32`. Change the internal
service methods to:

```rust
fn show_modal(&self, request_json: &str) -> Result<(), ServiceError>;
fn show_action_sheet(&self, request_json: &str) -> Result<(), ServiceError>;
fn open_app_authorize_setting(&self, request_id: i32) -> Result<(), ServiceError>;
```

Result hooks receive `requestId` first and settle exact matches. These are
engine-internal changes; the public Java callback handler interfaces listed in
the execution rules remain unchanged.

- [ ] **Step 5: Run resource and UI tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::resource_registry
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary::ui_result
cargo check -p migo-runtime-v8 -p migo-platform --locked --offline
```

Expected: PASS with exact out-of-order settlement and stale-event rejection.

- [ ] **Step 6: Commit resource and UI migration**

```bash
git add engine/crates/runtime-v8/src/ad/01_ad.js \
  engine/crates/runtime-v8/src/media/01_camera.js \
  engine/crates/runtime-v8/src/media/04_video.js \
  engine/crates/runtime-v8/src/audio/02_inner_audio_context.js \
  engine/crates/runtime-v8/src/ui/01_interaction.js \
  engine/crates/runtime-v8/src/ui/mod.rs \
  engine/crates/runtime-v8/src/system/02_authorize.js \
  engine/crates/runtime-v8/src/system/mod.rs \
  engine/crates/shared/src/services/interaction.rs \
  engine/crates/shared/src/services/system_info.rs \
  engine/crates/platform/src/android/services/mod.rs \
  engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs
git commit -m "runtime: bind native resources and ui results to host ids"
```

## Task 6: Make Every Android Result Echo Its ID

**Files:**
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/LocationProvider.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScanCodeManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ImageApiManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/VideoManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/InteractionUI.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/ResultProxyActivity.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeMethods.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeBridge.java`
- Modify: `engine/crates/platform/src/android/jni/inbound.rs`
- Modify: `engine/crates/platform/src/android/jni/outbound.rs`
- Modify: `engine/crates/platform/src/android/jni/registration.rs`
- Modify: `engine/crates/platform/src/android/jni/profile_contract.rs`
- Create: `platforms/android/library/src/test/java/com/migo/runtime/internal/CallbackCorrelationTest.java`

- [ ] **Step 1: Add failing Java correlation tests**

Test success and failure JSON builders for location, fuzzy location, scan,
compress image, choose image, and choose message file. Every output must contain
the exact positive ID from its options. Test modal, action sheet, Bluetooth
settings, and application-settings callback forwarding with two distinct IDs
completed in reverse order. Test `VideoManager` parsing with supplied ID `77`
and assert its returned JSON is `{"videoId":77}`.

- [ ] **Step 2: Run the Java test and verify failure**

```bash
cd platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*CallbackCorrelationTest'
```

Expected: FAIL because these paths currently omit IDs and `VideoManager`
allocates independently.

- [ ] **Step 3: Echo IDs through all JSON result paths**

Parse `requestId` once at the entry to each location, scan, compress, choose
image, and choose-message-file operation. Pass it into every success, error,
cancel, timeout, missing-activity, and exception result builder. A malformed or
non-positive request fails immediately without launching Android work.

For image-picker operations, store the request ID in the individual
`ResultProxyActivity.PendingRequest`, not one mutable manager field; concurrent
picker completions must not overwrite correlation.

Replace `sNextRequestCode.getAndIncrement() % 55000` with a positive,
non-wrapping `AtomicLong` token used only as the `_proxy_request_token` map key
and Intent extra. Android's actual `startActivityForResult` request code remains
the caller's existing bounded constant inside each proxy Activity. Allocation
at `Long.MAX_VALUE` fails before insertion. Never evict and reuse a token merely
because it is old; cancellation removes the exact generation-owned entry.

- [ ] **Step 4: Carry IDs through internal non-JSON callbacks**

Change the internal callback shapes to include `requestId`:

```java
onBluetoothSettingResult(int sessionId, int requestId, boolean enabled)
onAppAuthorizeSettingResult(int sessionId, int requestId, int code)
onModalResult(int sessionId, int requestId, int confirm, int cancel)
onActionSheetResult(int sessionId, int requestId, int tapIndex)
```

Update `NativeBridge`, JNI registration descriptors, `inbound.rs` hook argument
arrays, and outbound calls together. The JavaScript hooks receive the ID as
their first argument. Update `openSystemBluetoothSetting` to accept the ID all
the way from `op_open_system_bluetooth_setting(requestId)`.

- [ ] **Step 5: Make Android video honor the runtime ID**

In `VideoManager.create`, require `options.videoId` in
`1..=Integer.MAX_VALUE`, remove `nextVideoId`, and reject duplicate live IDs.
Keep the external JSON response shape unchanged.

- [ ] **Step 6: Run Java and JNI contract tests**

```bash
cd platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*CallbackCorrelationTest'
./gradlew :library:testFullDebugUnitTest
cd ../..
bash scripts/test-platform-v8-boundary-contract.sh
```

Expected: PASS; JNI descriptors, Java declarations, and Rust callbacks agree.

- [ ] **Step 7: Commit Android ID transport**

```bash
git add platforms/android/library/src/main/java/com/migo/runtime/internal/platform/LocationProvider.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScanCodeManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ImageApiManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/VideoManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/InteractionUI.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/ResultProxyActivity.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeMethods.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeBridge.java \
  platforms/android/library/src/test/java/com/migo/runtime/internal/CallbackCorrelationTest.java \
  engine/crates/platform/src/android/jni/inbound.rs \
  engine/crates/platform/src/android/jni/outbound.rs \
  engine/crates/platform/src/android/jni/registration.rs \
  engine/crates/platform/src/android/jni/profile_contract.rs
git commit -m "android: preserve runtime callback ids"
```

## Task 7: Add Android Generation Tokens And Restart Cleanup

**Files:**
- Create: `platforms/android/library/src/main/java/com/migo/runtime/internal/RuntimeGenerationBoundary.java`
- Create: `platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeGenerationBoundaryTest.java`
- Create: `platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeRestartCloseOrderingTest.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/GameSession.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/TerminalCleanupState.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeMethods.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeBridge.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/ResultProxyActivity.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/SensorExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NetworkExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/MediaExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/InputExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/BluetoothExports.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/LocationProvider.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/InteractionUI.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/AudioRecorderManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/CameraManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/VideoManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ImageApiManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScanCodeManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/DeviceSensorManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/NetworkMonitor.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/BluetoothManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/KeyboardManager.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScreenCaptureObserver.java`
- Modify: `engine/crates/platform/src/android/jni/inbound.rs`
- Modify: `engine/crates/platform/src/android/jni/outbound.rs`
- Modify: `engine/crates/platform/src/android/jni/registration.rs`
- Modify: `engine/crates/platform/src/android/jni/profile_contract.rs`
- Modify: `engine/crates/platform/src/android/services/mod.rs`
- Create: `engine/crates/shared/src/services/runtime_call.rs`
- Modify: `engine/crates/shared/src/services/mod.rs`
- Modify: `engine/crates/shared/src/services/ad.rs`
- Modify: `engine/crates/shared/src/services/camera.rs`
- Modify: `engine/crates/shared/src/services/device.rs`
- Modify: `engine/crates/shared/src/services/image_api.rs`
- Modify: `engine/crates/shared/src/services/interaction.rs`
- Modify: `engine/crates/shared/src/services/location.rs`
- Modify: `engine/crates/shared/src/services/network.rs`
- Modify: `engine/crates/shared/src/services/scan_code.rs`
- Modify: `engine/crates/shared/src/services/system_info.rs`
- Modify: `engine/crates/shared/src/services/video.rs`
- Modify: `engine/crates/shared/src/protocol/host_cmd.rs`
- Modify: `engine/crates/runtime-v8/src/base/mod.rs`
- Modify: `engine/crates/runtime-v8/src/ad/mod.rs`
- Modify: `engine/crates/runtime-v8/src/audio/ops.rs`
- Modify: `engine/crates/runtime-v8/src/device/mod.rs`
- Modify: `engine/crates/runtime-v8/src/input/mod.rs`
- Modify: `engine/crates/runtime-v8/src/media/mod.rs`
- Modify: `engine/crates/runtime-v8/src/system/mod.rs`
- Modify: `engine/crates/runtime-v8/src/ui/mod.rs`
- Modify: `engine/crates/core/src/runtime/host.rs`
- Modify: `engine/crates/core/src/runtime/registry.rs`
- Modify: `engine/crates/capi/src/keyboard.rs`
- Test: `engine/crates/core/src/runtime/restart_boundary.rs`
- Test: `engine/crates/core/src/runtime/registry.rs`
- Test: `engine/crates/capi/src/keyboard.rs`

- [ ] **Step 1: Write failing token, ad, and permission tests**

Assert that generation `1` tokens are current before
`beginRestart(sessionId, 1, 2)` and stale afterward; generation `2` tokens are
current only after `completeRestart(sessionId, 2)`. Assert a retired token
remains stale through later generations.
Reject wrong retired generation, non-increasing generation, and missing
session without mutating state.

Use a controllable completion future for camera/recorder/Bluetooth teardown.
Assert the fence does not complete and generation `2` acquisition is rejected
until all three futures complete. Assert an exceptional completion retains the
failed handle, reports the terminal cleanup error, and never admits generation
`2` acquisition. Advance a diagnostic clock beyond five seconds and assert the
same barrier is still owned and pending, the restart is permanently terminal,
and elapsed time has not released it. Then complete the retained cleanup
future: ownership reaches quiescence, but generation `2` never becomes
`ACTIVE`, `completeRestart` is not called, the commit waiter returns terminal
failure, and no candidate is published.

Call `preflightRestart(sessionId, 1, 2)` with an invalid generation and an
injected platform-readiness failure. Assert generation `1` remains `ACTIVE`,
its tokens and resource sets are unchanged, and acquisition still succeeds.

Register two active ad IDs and two pending permission IDs. Fence restart and
assert the returned cleanup snapshot contains both ads, both permission IDs are
retired, and the session-level handler objects supplied by the test remain
registered. Assert a stale ad event, permission resolve/fail, and `setScope`
return false while new-generation equivalents return true.

Add a latch-controlled TOCTOU test: an ID-less callback observes its Java token
as current, pauses, restart commits generation `2`, then the callback proceeds
to native enqueue carrying generation `1`. In a core test named
`retired_callback_generation_is_dropped_at_host_dispatch`, queue the command,
advance the test Host's `RestartBoundary` from `1` to `2` before dispatch, and
assert no hook fires; the otherwise identical generation `2` command fires
once. Tasks 10 and 11 add the full queued-behind-`Restart` interleaving after
the transaction exists.

Repeat the Host-dispatch test with one ID-bearing result: queue its exact ID and
captured generation `1`, advance to generation `2`, and assert the replacement
runtime's hook is never invoked. The corresponding generation `2` result must
enter the hook once and settle only its exact ID. This test prevents a fresh
runtime's empty pending map from being used as the first correctness check.

Create a Worker with immutable generation `1`, block its resource-acquisition
call immediately before the Android per-session lock, commit generation `2`,
then release the Worker. Assert `requireActive(sessionId, 1)` rejects the
request and no manager or native handle is created. Also assert all retired
Workers are stopped and joined before candidate publication.

In `RuntimeRestartCloseOrderingTest`, block the Host-side abort waiter on one
retained Android cleanup handle. Invoke terminal Session close and assert it
calls the shared restart-cleanup retry before native shutdown. Make that retry
fail once and assert `NativeMethods.shutdown` and Host join are not invoked and
the Session remains registered. On a later explicit close, release the handle,
assert the abort waiter completes, and only then observe shutdown/join. Use
latches and recorded calls, never sleeps.

Add `soft_keyboard_commands_capture_ingress_generation` in the C API keyboard
tests. Pass generation `7` through the extracted conversion helper, build all
four commands, and assert each carries `7`; repeat one command with `8` and
assert it carries `8`. Extend the core keyboard-reserve test to read its
fixture's generation and prove reserving a terminal command preserves that
stamp.

- [ ] **Step 2: Run the generation tests and verify failure**

```bash
cd platforms/android
./gradlew :library:testFullDebugUnitTest \
  --tests '*RuntimeGenerationBoundaryTest' \
  --tests '*RuntimeRestartCloseOrderingTest'
cd ../../engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core retired_callback_generation_is_dropped_at_host_dispatch
bash ../scripts/run-cargo-lib-test-filter.sh migo-capi soft_keyboard_commands_capture_ingress_generation
```

Expected: FAIL because no per-session generation boundary, Host dispatch
comparison, shared restart cleanup retry, or close-before-join ordering exists.

- [ ] **Step 3: Implement atomic per-session logical revocation**

`RuntimeGenerationBoundary` owns a `ConcurrentHashMap<Integer, SessionState>`.
Synchronize each `SessionState` while checking/updating:

```java
static final class Token {
    final int sessionId;
    final long generation;
    boolean isCurrent() { return RuntimeGenerationBoundary.isCurrent(this); }
}

static void preflightRestart(int sessionId, long retired, long next)
static RestartCleanup beginRestart(int sessionId, long retired, long next)
static void completeRestart(int sessionId, long next)
static RestartCleanup beginAbort(int sessionId, long generation)
static Token requireActive(int sessionId, long expectedGeneration)
```

`requireActive` validates `phase == ACTIVE`, `current == expectedGeneration`,
and a positive generation while holding the same `SessionState` lock used by
restart. It never substitutes the current generation for the caller's value.
Add `RuntimeCallContext { host_id: i32, runtime_generation: i64 }` in
`shared::services::runtime_call`. Every runtime-created resource acquisition
constructs it from the calling main runtime or Worker's immutable
`HostOpState`, passes it through the exact shared service method, Android
service implementation, JNI outbound method, and Java factory, and calls
`requireActive(sessionId, expectedGeneration)` at the acquisition point. This
exact match is required even if an earlier Rust or Java check observed the
Session as active. The Task 7 file list is the closed audit surface for these
signature changes; the static gate in Task 11 fails when any resource-creating
op bypasses the context.

`beginRestart` validates `current == retired`, `next == retired + 1`, and
`next > 0`; then it marks the session `RESTARTING`, invalidates the retired
token, and atomically detaches active-ad and pending-permission sets. It returns
immutable snapshots for physical cleanup. `completeRestart` publishes `next`
and marks the session `ACTIVE` only after every required completion barrier
succeeds. `failRestart` leaves acquisition closed. No callback invocation or
Android UI work occurs while holding the state lock.

`beginAbort` accepts the active matching generation or the outstanding next
generation while `RESTARTING`; it leaves or moves the session to `RESTARTING`,
invalidates that generation, and atomically detaches its active sets. It is
idempotent for the same generation so cleanup can finish during terminal Host
teardown without admitting resource acquisition.

`NativeExports.registerSession` calls `registerSession(sessionId, 1)` and
rejects duplicate registration. Terminal `unregisterSession` removes the
boundary only after terminal manager cleanup.

`preflightRestart` performs the same generation/session validation plus a
non-mutating check that the main-thread completion executor is accepting work.
It does not change phase, tokens, active sets, managers, or UI.

- [ ] **Step 4: Bind UI, activity results, and location to tokens**

Capture a `Token` for each dialog, `ResultProxyActivity.PendingRequest`, and
location listener/timeout. Add
`ResultProxyActivity.cancelSessionGeneration(sessionId, retired)` and
`LocationProvider.cancelSessionGeneration(sessionId, retired)`; both remove
exact pending entries before scheduling platform cleanup. Add
`InteractionUI.destroySessionGeneration` to dismiss modal, action-sheet,
toast, and loading UI and invalidate their click closures. Test each stale
closure after restart and one current closure after completion.

- [ ] **Step 5: Bind auth and subpackage completions to tokens**

At entry to `NativeExports.login`, `checkSession`, `getUserInfo`,
`getPhoneNumber`, and `subpackageDownload`, obtain the current token and
capture it in every public-handler callback object. Immediately before each
progress, success, failure, or exception result calls `NativeMethods`, return
if that captured token is stale. Never fetch a replacement current token in a
completion. Keep the original `requestId` in every result JSON. Test a callback
that completes after restart and a current-generation callback for each auth
method plus subpackage progress and terminal completion.

- [ ] **Step 6: Bind recorder, media, sensors, and network to tokens**

Capture one token when `AudioRecorderManager`, `CameraManager`, `VideoManager`,
`ImageApiManager`, `DeviceSensorManager`, `ScreenCaptureObserver`, and
`NetworkMonitor` are created. Every callback uses that captured token and every
`destroy()` flips its destroyed flag before unregistering listeners. Test a
stale recorder frame, camera/video event, sensor sample, screen-capture event,
and network change; none may reach `NativeMethods`.

- [ ] **Step 7: Bind Bluetooth and keyboard managers to tokens**

Capture one token when `BluetoothManager` and `KeyboardManager` are created.
Gate adapter/device discovery, beacon, BLE connection/value/MTU, keyboard
input/confirm/complete/height callbacks with the captured token. Test one stale
and one current callback for each manager.

Every manager factory calls
`RuntimeGenerationBoundary.requireActive(sessionId, expectedGeneration)`
immediately before acquiring hardware and receives the token it stores. The
expected generation originates in the calling main runtime or Worker's
`HostOpState`; no layer reloads a newer generation. Camera, recorder, location,
sensor, Bluetooth discovery/beacon/BLE/GATT teardown returns a
`CompletionStage<Void>` that completes only after its listener is unregistered,
worker stopped, device or recorder closed, and manager map entry removed.
Posting a main-thread operation is not completion.

- [ ] **Step 8: Carry generation through every runtime-owned callback**

Change the internal Java/native callback signatures for every ID-bearing
result listed in the callback-hook audit and for recorder event/frame,
device motion, gyroscope, device orientation, compass, accelerometer, network
status, Bluetooth adapter/device/beacon/BLE/MTU, keyboard input/confirm/
complete/height, and screen-capture events to include `long generation`
immediately after `sessionId`. Each manager passes its captured token's value;
`NativeMethods` may return early when `token.isCurrent()` is false, but always
passes the captured value rather than reading the current generation again.

For an ID-bearing result, place `long generation` next to its request/resource
ID and preserve both unchanged across Java, `NativeBridge`, JNI, and the queued
command. For an ID-less result, place it immediately after `sessionId`. Add
`runtime_generation: i64` to every runtime-owned callback `HostCommand`,
including all commands reached by the 25 ID-bearing hooks and these ID-less
variants:

```text
OnDeviceMotionChange, OnGyroscopeChange, OnDeviceOrientationChange,
OnCompassChange, OnAccelerometerChange, OnNetworkStatusChange,
RecorderEvent, RecorderFrameData, OnBluetoothAdapterStateChange,
OnBluetoothDeviceFound, OnBeaconUpdate, OnBeaconServiceChange,
OnBLEConnectionStateChange, OnBLECharacteristicValueChange, OnBLEMTUChange,
OnKeyboardInput, OnKeyboardConfirm, OnKeyboardComplete,
OnKeyboardHeightChange, OnUserCaptureScreen
```

Implement `HostCommand::callback_generation() -> Option<i64>` as one exhaustive
match over every runtime-owned callback command. At the start of
`Host::handle_command_inner`, return without invoking JS when that value differs
from the acquire-loaded `self.restart_boundary.current()`. Reject non-positive
JNI generations before enqueue. ID-bearing result/resource commands therefore
have two independent checks in the required order: Host generation before JS
dispatch, then exact ID lookup inside the current runtime.

All existing struct variants above gain a `runtime_generation: i64` field.
Convert the two non-struct variants exactly as follows so construction and
matching cannot omit the generation:

```rust
OnBLECharacteristicValueChange {
    data: Box<BleCharacteristicData>,
    runtime_generation: i64,
},
OnUserCaptureScreen {
    runtime_generation: i64,
},
```

Android ingress always uses the positive captured Java generation. The C API
soft-keyboard entry point is direct current Host input, not a runtime-created
manager callback: change `validated_keyboard_to_command` to accept the acquire
loaded `ingress.runtime_generation()` and stamp it into the four keyboard
variants before enqueue. Update its unit tests and the keyboard reserve tests
in `runtime/registry.rs`. This preserves the public C ABI and ensures a C event
queued before restart commit carries the retired value and is dropped.

- [ ] **Step 9: Track ads and permission requests**

In `NativeExports.adCreate`, add the positive ID to the current generation
before invoking the handler; remove it if handler creation throws. In
`adDestroy`, remove the ID before invoking `destroyAd`. Every
`SessionAdEventSink.emit` returns unless the ID is active in the current
generation.

Replace the shared stateless permission sink with a sink created from the
request's generation token. Add the positive ID before
`PermissionHandler.requestScope`; exact resolve/fail consumes it once. A stale
sink cannot call `setScope`, resolve, or fail. Do not clear Rust's already
committed scope cache on restart.

- [ ] **Step 10: Add restart-only cleanup without clearing public handlers**

Add `NativeExports.preflightRuntimeRestart(sessionId, retired, next)` as a
read-only wrapper over `preflightRestart`. Add
`NativeExports.commitRuntimeRestart(sessionId, retired, next)`; it first calls
`beginRestart`, then:

```text
SensorExports.destroyAll
NetworkExports.destroyAll
MediaExports.destroyAll
InputExports.destroyAll
BluetoothExports.destroyAll
InteractionUI.destroySessionGeneration
ResultProxyActivity.cancelSessionGeneration
LocationProvider.cancelSessionGeneration
AdHandler.destroyAd for every detached active ad
```

Aggregate the completion stages for exclusive/power-sensitive managers in one
owned cleanup barrier. A five-second threshold may emit one diagnostic and a
terminal error callback, but it must not complete, cancel, detach, or abandon
the barrier. Crossing the threshold permanently moves the restart to a
cleanup-only terminal state. Later handle release completes ownership cleanup
and unblocks abort/termination, but must return terminal failure to the commit
waiter; it can never call `completeRestart`, activate the next generation, or
publish the candidate. Exceptional cleanup follows the same rule, retains the
concrete failed handles, and permits explicit terminal close to retry them.
`completeRestart` is reachable only when every handle was released and every
listener/worker quiesced before any terminal cleanup condition occurred.
Non-exclusive image-compression,
auth, and subpackage work is not awaited because its token has already been
invalidated and it owns no exclusive native resource.

Add `NativeExports.abortRuntimeRestart(sessionId, generation)`. It calls
`beginAbort`, performs the same active-ad, pending-permission, manager, UI,
activity-result, location, and exclusive-resource cleanup for the unpublished
candidate, and leaves the session closed to runtime acquisition. Abort uses the
same owned, retryable cleanup state and has no timeout escape. It retains
Session handler registrations until terminal Session destruction.

Expose `NativeExports.retryRuntimeRestartCleanup(sessionId)` as the only
caller-driven retry of that same retained cleanup state. Refactor
`GameSession.close()` and `TerminalCleanupState` into ordered phases: this
restart-cleanup retry is a hard prerequisite; only after it succeeds may close
call `NativeMethods.shutdown(sessionId)` and join the Host, then clean remaining
managers, and finally release Session/RuntimeRegistry/temp ownership. The
prerequisite must not sit in a best-effort `runAll` group with native shutdown,
because continuing to the join after it fails deadlocks with the Host waiting
in abort. On failure, keep `sSessions` and every failed handle, report
`ERR_CLEANUP_FAILED`, and let a later explicit `GameSession.close()` retry the
prerequisite. Add latch tests proving shutdown is never invoked while abort is
pending, a failed retry leaves shutdown count zero, and a later successful
retry releases abort before shutdown/join begins.

The method must retain `sSessions`, `sErrorCallbacks`, `sAuthHandlers`,
`sPermissionHandlers`, `sAdHandlers`, `sGameLogHandlers`,
`sSubpackageHandlers`, `sMessageHandlers`, and standing native permission
decisions. Keep terminal `destroyAllManagers` responsible for clearing those
registrations and removing the generation state.

- [ ] **Step 11: Run Android fence tests**

```bash
cd platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*RuntimeGenerationBoundaryTest'
./gradlew :library:testFullDebugUnitTest --tests '*PermissionRevocationTest'
./gradlew :library:testFullDebugUnitTest
cd ../../engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core retired_callback_generation_is_dropped_at_host_dispatch
bash ../scripts/run-cargo-lib-test-filter.sh migo-core key_up_and_keyboard_complete_each_use_the_reliable_reserve
bash ../scripts/run-cargo-lib-test-filter.sh migo-capi soft_keyboard_commands_capture_ingress_generation
cargo check -p migo-core -p migo-platform -p migo-capi --locked --offline
```

Expected: PASS; restart preserves handler identity, drops stale callbacks,
blocks new hardware acquisition until quiescence, and terminal destruction
still removes all session state.

- [ ] **Step 12: Commit Android generation fencing**

```bash
git add platforms/android/library/src/main/java/com/migo/runtime/internal/RuntimeGenerationBoundary.java \
  platforms/android/library/src/main/java/com/migo/runtime/GameSession.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/TerminalCleanupState.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeMethods.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NativeBridge.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/ResultProxyActivity.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/SensorExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/NetworkExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/MediaExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/InputExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/BluetoothExports.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/LocationProvider.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/InteractionUI.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/AudioRecorderManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/CameraManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/VideoManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ImageApiManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScanCodeManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/DeviceSensorManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/NetworkMonitor.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/BluetoothManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/KeyboardManager.java \
  platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ScreenCaptureObserver.java \
  platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeGenerationBoundaryTest.java \
  platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeRestartCloseOrderingTest.java \
  engine/crates/platform/src/android/jni/inbound.rs \
  engine/crates/platform/src/android/jni/outbound.rs \
  engine/crates/platform/src/android/jni/registration.rs \
  engine/crates/platform/src/android/jni/profile_contract.rs \
  engine/crates/platform/src/android/services/mod.rs \
  engine/crates/shared/src/services/runtime_call.rs \
  engine/crates/shared/src/services/mod.rs \
  engine/crates/shared/src/services/ad.rs \
  engine/crates/shared/src/services/camera.rs \
  engine/crates/shared/src/services/device.rs \
  engine/crates/shared/src/services/image_api.rs \
  engine/crates/shared/src/services/interaction.rs \
  engine/crates/shared/src/services/location.rs \
  engine/crates/shared/src/services/network.rs \
  engine/crates/shared/src/services/scan_code.rs \
  engine/crates/shared/src/services/system_info.rs \
  engine/crates/shared/src/services/video.rs \
  engine/crates/shared/src/protocol/host_cmd.rs \
  engine/crates/runtime-v8/src/base/mod.rs \
  engine/crates/runtime-v8/src/ad/mod.rs \
  engine/crates/runtime-v8/src/audio/ops.rs \
  engine/crates/runtime-v8/src/device/mod.rs \
  engine/crates/runtime-v8/src/input/mod.rs \
  engine/crates/runtime-v8/src/media/mod.rs \
  engine/crates/runtime-v8/src/system/mod.rs \
  engine/crates/runtime-v8/src/ui/mod.rs \
  engine/crates/core/src/runtime/host.rs \
  engine/crates/core/src/runtime/registry.rs \
  engine/crates/core/src/runtime/restart_boundary.rs \
  engine/crates/capi/src/keyboard.rs
git commit -m "android: fence callbacks and resources on runtime restart"
```

## Task 8: Add Ordered Audio Reset

**Files:**
- Modify: `engine/crates/shared/src/protocol/audio_cmd.rs`
- Modify: `engine/crates/core/src/services/audio.rs`
- Modify: `engine/crates/audio/src/audio_thread.rs`
- Test: `engine/crates/audio/src/audio_thread.rs`
- Test: `engine/crates/core/src/services/audio.rs`

- [ ] **Step 1: Write failing reset-barrier tests**

Populate contexts, node index, inner players, media players, streaming state,
and an in-flight decode destination. Send `ResetForRuntimeRestart`, wait for its
response, then send new-generation create commands reusing deliberately chosen
test IDs. Assert all retired maps are empty before the response, no retired
event is emitted, late decode output misses its retired destination, and the
new commands succeed only after the barrier.

Add an `AudioService` test for a lazy, not-yet-started audio thread: buffered
old commands are discarded by reset without starting hardware output.

- [ ] **Step 2: Run audio tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-audio restart_reset
bash ../scripts/run-cargo-lib-test-filter.sh migo-core audio_restart_reset
```

Expected: FAIL because `PauseAll` retains every registry.

- [ ] **Step 3: Add the ordered command and service method**

Add:

```rust
AudioCmd::ResetForRuntimeRestart {
    generation: i64,
    resp: AudioResp<()>,
}
```

`AudioService::reset_for_runtime_restart(generation)` sends the command and
awaits its oneshot. For a lazy service, discard buffered/runtime-channel audio
commands and record the new generation without starting the thread.

The audio-thread arm pauses output, clears contexts and `node_index`, stops and
clears inner players, clears media players, invalidates decode destinations,
resets power/stream state, drains pending player events, stores the new
generation, and only then sends `Ok(())`. Keep immutable decoded-audio cache and
the worker pool; late decode output is ignored because its destination belongs
to the retired generation.

- [ ] **Step 4: Run audio tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-audio restart_reset
bash ../scripts/run-cargo-lib-test-filter.sh migo-core audio_restart_reset
cargo check -p migo-audio -p migo-core --locked --offline
```

Expected: PASS with the response proving the clear happened before new work.

- [ ] **Step 5: Commit audio reset**

```bash
git add engine/crates/shared/src/protocol/audio_cmd.rs \
  engine/crates/core/src/services/audio.rs \
  engine/crates/audio/src/audio_thread.rs
git commit -m "audio: reset runtime resources at restart barrier"
```

## Task 9: Add Ordered Render Reset

**Files:**
- Modify: `engine/crates/shared/src/protocol/render_cmd.rs`
- Modify: `engine/crates/core/src/services/render.rs`
- Modify: `engine/crates/graphics/src/render_thread.rs`
- Modify: `engine/crates/graphics/src/canvas/manager/mod.rs`
- Modify: `engine/crates/graphics/src/canvas/handler.rs`
- Test: `engine/crates/graphics/src/canvas/manager/mod.rs`
- Test: `engine/crates/graphics/src/render_thread.rs`

- [ ] **Step 1: Write failing render-barrier tests**

Create an onscreen canvas, offscreen canvas, Canvas2D state, WebGL buffer/
texture/program IDs, pending upload, and image. Send the restart-reset command
and await it. Assert offscreen canvases, WebGL/Canvas2D resource tables,
snapshots, pending uploads, and old images are gone; the compatible native
Surface lease and onscreen dimensions remain owned; the onscreen canvas is
transparent and uses fresh default GL/Canvas2D state. Assert old queued draw
commands cannot mutate the post-barrier canvas.

- [ ] **Step 2: Run render tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-graphics runtime_restart_reset
```

Expected: FAIL because restart currently destroys only shared image IDs.

- [ ] **Step 3: Add the must-deliver reset command**

Add `RenderCommand::ResetForRuntimeRestart { generation, resp }` and
`RenderService::reset_for_runtime_restart(generation)`. Classify it with
must-deliver lifecycle controls; do not place one command per GL object on the
bounded draw queue.

On the render thread, finish/cancel pending uploads, destroy all offscreen and
JavaScript-owned resource registries, discard saved Canvas2D/WebGL restoration
state, and rebuild the onscreen share group into fresh defaults using the
currently installed compatible target. Reuse the teardown primitives behind
`CanvasManager::tear_down_share_group`, but do not restore the retired
`ShareGroupRestorePlan`. Preserve native target ownership and Surface
generation. Send the response only after the new blank onscreen state is
current.

- [ ] **Step 4: Run render tests and checks**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-graphics runtime_restart_reset
bash ../scripts/run-cargo-lib-test-filter.sh migo-graphics canvas
cargo check -p migo-graphics -p migo-core --locked --offline
```

Expected: PASS without Surface release or stale GL-resource reuse.

- [ ] **Step 5: Commit render reset**

```bash
git add engine/crates/shared/src/protocol/render_cmd.rs \
  engine/crates/core/src/services/render.rs \
  engine/crates/graphics/src/render_thread.rs \
  engine/crates/graphics/src/canvas/manager/mod.rs \
  engine/crates/graphics/src/canvas/handler.rs
git commit -m "render: reset isolate resources at restart barrier"
```

## Task 10: Make Restart A Cross-Platform Transaction

**Files:**
- Modify: `engine/crates/core/src/services/platform.rs`
- Modify: `engine/crates/core/src/services/mod.rs`
- Modify: `engine/crates/core/src/lib.rs`
- Modify: `engine/crates/core/src/runtime/mod.rs`
- Modify: `engine/crates/core/src/runtime/restart_boundary.rs`
- Modify: `engine/crates/core/src/runtime/host.rs`
- Modify: `engine/crates/core/src/runtime/thread.rs`
- Modify: `engine/crates/runtime-v8/src/host_runtime.rs`
- Modify: `engine/crates/runtime-v8/src/worker/mod.rs`
- Modify: `engine/crates/platform/src/android/platform.rs`
- Modify: `engine/crates/platform/src/android/jni/outbound.rs`
- Modify: `engine/crates/platform/src/android/jni/profile_contract.rs`
- Modify: `engine/crates/platform/src/linux/platform.rs`
- Modify: `engine/crates/platform/src/windows/platform.rs`
- Modify: `engine/crates/capi/src/host_kit.rs`
- Test: `engine/crates/core/src/runtime/restart_boundary.rs`
- Test: `engine/crates/core/src/runtime/tests/thread_wiring.rs`
- Test: `engine/crates/platform/src/android/platform.rs`

- [ ] **Step 1: Write failing lifecycle-hook and transaction tests**

Use a fake platform to record
`preflight_runtime_restart(host, retired, next)` followed by
`commit_runtime_restart(host, retired, next)`. Assert both receive `1, 2`, and
commit runs before old-runtime drop and candidate publication. Inject a
preflight failure and assert the old isolate remains installed, its generation
stays `1`, and paused services resume. Inject commit, audio reset, render reset,
candidate construction, prelude, and module-evaluation failures; each must
leave no replacement isolate published, invoke abort cleanup, and return a
terminal Host disposition only after abort cleanup has quiesced. Inject one
cleanup-attempt failure, retain its handle, retry it through terminal Session
close, and assert the Host thread remains in cleanup-only state until the
shared barrier completes. Assert commit failure never resumes the old isolate.
Assert a successful path commits generation `2` only after module evaluation.
Queue an ID-less generation `1` event while commit is blocked, then release
commit and publish generation `2`; assert Host dispatch drops the queued event.
This pins `Host.restart_boundary.current()` as the authority rather than
Java's token state.

Add a latch-controlled Worker ownership test. Keep one retired Worker blocked
after restart has requested termination; assert the candidate-publication latch
does not fire while the Worker join is incomplete. Release the Worker and
assert its owned `JoinHandle` is consumed before publication. A Worker panic or
join failure is terminal, and dropping a `JoinHandle` to detach the thread is
forbidden.

- [ ] **Step 2: Run focused core/platform tests and verify failure**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-platform restart_fence
```

Expected: FAIL because there is no platform hook or transactional publication.

- [ ] **Step 3: Add the focused platform capability**

Add and re-export:

```rust
pub trait RuntimeLifecycle: Send + Sync {
    fn preflight_runtime_restart(
        &self,
        _host_id: i32,
        _retired_generation: i64,
        _next_generation: i64,
    ) -> shared::error::EngineResult<()> {
        Ok(())
    }

    fn commit_runtime_restart(
        &self,
        _host_id: i32,
        _retired_generation: i64,
        _next_generation: i64,
    ) -> shared::error::EngineResult<()> {
        Ok(())
    }

    fn abort_runtime_restart(
        &self,
        _host_id: i32,
        _candidate_generation: i64,
    );
}
```

Require `RuntimeLifecycle` in `PlatformServices`. Implement it explicitly for
Android, Linux, Windows, and `CapiHostKit`. Linux, Windows, and the current C
Host Kit implement immediate abort completion because their supplied services
create no asynchronous result/resource streams; their tests pin that
assumption. Android preflight
calls `NativeExports.preflightRuntimeRestart(IJJ)V`; commit calls
`NativeExports.commitRuntimeRestart(IJJ)V`; both convert JNI lookup, invocation,
and Java exception failures into `EngineError`. Its abort hook calls
`NativeExports.abortRuntimeRestart(IJ)V`. Once preflight succeeds, abort is an
infallible ownership barrier at this trait boundary: implementations report an
operational cleanup failure to the host, retain the failed handle, and keep the
call pending until an explicit terminal cleanup retry reaches quiescence. They
do not return an error that would let the Host thread abandon owned resources.

- [ ] **Step 4: Make generation transition checked and non-wrapping**

Use only the `RestartBoundary` API completed and tested in Task 2. Read
`retired = current()`, compute `candidate = candidate_generation()` before
preflight, and call `commit(retired, candidate)` only after candidate module
evaluation and installation succeed. Exhaustion occurs before revocation and
is an ordinary preflight failure; a compare-exchange failure occurs after the
irreversible platform commit and is terminal. Runtime generations remain positive Rust
`i64`/JNI `jlong`/Java `long` values; introduce no narrowing cast, second
counter, reset, direct store, or rollback path.

- [ ] **Step 5: Refactor module loading for an unpublished candidate**

Extract the prelude and module-evaluation body so it accepts
`&mut HostJsRuntime`. During restart, construct `new_js` as a local variable,
run preludes and `evaluate_module` on it, and call `self.js.set(new_js)` only
after all return `Ok(())`. Move `notify_game_ready`, generation commit,
surface restore, render/audio resume, and context-loss reconciliation after
that set.

- [ ] **Step 6: Sequence the restart fence and reset barriers**

Define the command-loop result and armed cleanup guard explicitly in
`runtime/host.rs`:

```rust
pub(crate) enum HostCommandDisposition {
    Continue,
    Terminate,
}

struct RestartAttemptGuard {
    platform: Arc<dyn PlatformServices>,
    host_id: i32,
    candidate_generation: i64,
    armed: bool,
}
```

`RestartAttemptGuard::disarm` sets `armed = false`. Its `Drop` implementation
synchronously calls
`platform.abort_runtime_restart(host_id, candidate_generation)` exactly once
when still armed; the call cannot return until the shared platform cleanup
barrier is quiescent. Change
`Host::handle_command` to return `HostCommandDisposition`; ordinary success,
ordinary command errors, and restart preflight failure return `Continue`, while
restart commit or later failure returns `Terminate`. Both dispatch sites in
`runtime/thread.rs` must break their Host loop on `Terminate`.

Use this exact order in `Host::on_restart`:

```text
pause render/audio and reset accepted input
compute next generation without committing it
call platform.preflight_runtime_restart(retired, next)
arm RestartAttemptGuard
call platform.commit_runtime_restart(retired, next)
stop and join every Worker owned by the retired runtime
begin the new RAF session ticket
close the old IO scheduler
await audio reset(next)
await render reset(next), including shared images
drop the old isolate
construct and fully evaluate the local candidate isolate
install the candidate and commit next generation
restore Surface, resume render/audio, reconcile context loss, notify ready
```

If read-only preflight fails, resume the old services and retain the old
runtime. Commit is the irreversible boundary: commit failure or any later
failure returns a terminal `HostCommandDisposition`; `runtime/thread.rs` exits
the Host loop and performs normal teardown rather than processing another
command with an empty or partial isolate. An armed `RestartAttemptGuard`
synchronously calls
`platform.abort_runtime_restart(host_id, candidate_generation)` on every
post-fence error or unwind; disarm it only after isolate installation and
generation commit. A cleanup-attempt failure keeps the Java Session in its
cleanup-only terminal state, reports through the Session error callback, and
leaves the synchronous abort call pending until an explicit
`GameSession.close()` retry releases the retained handle. Android and every
other platform reach candidate quiescence before the Host thread exits.

Implement the Worker line through an explicit
`HostJsRuntime::stop_and_join_workers()` API. It takes every `WorkerHandle` out
of the retired runtime, requests `force_terminate`, and consumes each owned
`JoinHandle` with `join`; it does not rely on `WorkerHandle::drop`, which would
detach the thread. The barrier has no timeout escape. A panic or failed join is
a post-commit terminal error and still leaves no live owned Worker before Host
exit or candidate publication.

- [ ] **Step 7: Run transaction and platform tests**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-core runtime::tests::thread_wiring
bash ../scripts/run-cargo-lib-test-filter.sh migo-platform restart_fence
bash ../scripts/run-cargo-lib-test-filter.sh migo-capi host_kit
cargo check -p migo-core -p migo-platform -p migo-capi --locked --offline
```

Expected: PASS; a post-fence error exits and no candidate is observable.

- [ ] **Step 8: Commit transactional restart**

```bash
git add engine/crates/core/src/services/platform.rs \
  engine/crates/core/src/services/mod.rs \
  engine/crates/core/src/lib.rs \
  engine/crates/core/src/runtime/mod.rs \
  engine/crates/core/src/runtime/restart_boundary.rs \
  engine/crates/core/src/runtime/host.rs \
  engine/crates/core/src/runtime/thread.rs \
  engine/crates/runtime-v8/src/host_runtime.rs \
  engine/crates/runtime-v8/src/worker/mod.rs \
  engine/crates/platform/src/android/platform.rs \
  engine/crates/platform/src/android/jni/outbound.rs \
  engine/crates/platform/src/android/jni/profile_contract.rs \
  engine/crates/platform/src/linux/platform.rs \
  engine/crates/platform/src/windows/platform.rs \
  engine/crates/capi/src/host_kit.rs
git commit -m "runtime: fence and publish restarts transactionally"
```

## Task 11: Add Race Regression And Exhaustive Audit Gates

**Files:**
- Modify: `engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs`
- Modify: `engine/crates/core/src/runtime/restart_boundary.rs`
- Modify: `platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeGenerationBoundaryTest.java`
- Create: `scripts/test-runtime-restart-generation-contract.sh`

- [ ] **Step 1: Add failing race tests**

Exercise these interleavings with latches rather than sleeps:

```text
old callback queued before Restart -> FIFO handles it only in old runtime
Restart dequeued -> old callback queues before commit -> captured generation makes post-commit Host dispatch drop it
Java token check returns current -> commit advances generation -> JNI enqueues captured old generation -> Host drops it
commit returns -> old callback attempts JNI -> dropped before Host channel
new request/resource is created -> old ID/event cannot find it
new callback with current ID/token -> delivered exactly once
old Worker allocates while replacement main runtime allocates -> IDs remain distinct
old Worker attempts resource acquisition after generation commit -> exact expected-generation check rejects it
restart commit stops and joins every old Worker before candidate publication
```

Also force allocator exhaustion in generic deferred, login, payment,
subpackage, ad, camera, video, and inner-audio constructors. Assert no platform
call, pending entry, or native resource appears.

- [ ] **Step 2: Run race tests and verify any missing assertion fails**

```bash
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*RuntimeGenerationBoundaryTest'
```

Expected: FAIL until every callback family and race phase is covered.

- [ ] **Step 3: Add the static audit gate**

The script must fail when any of these are present:

```text
base/02_async.js: local _nextId or pending.values().next fallback
system/13_login.js, payment/01_payment.js, base/04_subpackage.js: local request counters
ad/01_ad.js, media/01_camera.js, media/04_video.js: local callback/resource counters
audio/02_inner_audio_context.js: local callback-routed resource counter
ui/01_interaction.js or system/02_authorize.js: shift()-based result settlement
VideoManager.java: nextVideoId
```

It must positively require an exact Host-allocated request, ad, camera, or
resource ID on every ID-bearing `_internalOn*` hook enumerated in the audit,
and an explicit captured generation on every ID-bearing and ID-less hook.
Require `long generation` in `NativeMethods` and `NativeBridge`, `jlong` in JNI
ingress, `runtime_generation: i64` in every corresponding `HostCommand`, and
`HostCommand::callback_generation()` comparison with
`Host.restart_boundary.current()` before JS dispatch. Require every
resource-creation path to carry the caller's expected generation into the
platform's atomic `requireActive` check. A Java `token.isCurrent()` check or a
fresh runtime's exact-ID lookup alone must not satisfy the script. The script
must also require Android restart to preserve every Session-level handler map
named in Task 7.

- [ ] **Step 4: Run the race and audit gates**

```bash
bash scripts/test-runtime-restart-generation-contract.sh
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 runtime_restart_boundary
bash ../scripts/run-cargo-lib-test-filter.sh migo-core restart_boundary
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*RuntimeGenerationBoundaryTest'
```

Expected: PASS with no sleeps and no success based solely on source text.

- [ ] **Step 5: Commit regression gates**

```bash
git add scripts/test-runtime-restart-generation-contract.sh \
  engine/crates/runtime-v8/src/tests/runtime_restart_boundary.rs \
  engine/crates/core/src/runtime/restart_boundary.rs \
  platforms/android/library/src/test/java/com/migo/runtime/internal/RuntimeGenerationBoundaryTest.java
git commit -m "test: cover runtime restart callback races"
```

## Task 12: Regenerate Snapshot Identities And Run Regression Suites

**Files:**
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-full-aarch64.bin`
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-full-x86_64.bin`
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-aarch64.bin`
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-x86_64.bin`
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-aarch64.bin`
- Regenerate: `engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin`
- Regenerate: the `.manifest.json` paired with each snapshot above
- Test: `engine/crates/runtime-v8/src/tests/snapshot_fingerprint.rs`

- [ ] **Step 1: Prove committed snapshots are stale**

```bash
bash scripts/check-snapshot-freshness.sh --snapshot-kind host --product-profile full aarch64 x86_64
bash scripts/check-snapshot-freshness.sh --snapshot-kind host --product-profile slim aarch64 x86_64
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full aarch64 x86_64
```

Expected: FAIL/STALE because the base op table and embedded extension JS changed.

- [ ] **Step 2: Regenerate all host and Worker snapshots on matching Android ABIs**

Set `MIGO_ARM64_SERIAL` and `MIGO_X86_64_SERIAL` to explicit entries printed by
`adb devices`, then validate their ABIs before generation:

```bash
: "${MIGO_ARM64_SERIAL:?set MIGO_ARM64_SERIAL to one explicit adb device serial}"
: "${MIGO_X86_64_SERIAL:?set MIGO_X86_64_SERIAL to one explicit adb emulator serial}"
test "$(adb -s "$MIGO_ARM64_SERIAL" shell getprop ro.product.cpu.abi | tr -d '\r')" = arm64-v8a
test "$(adb -s "$MIGO_X86_64_SERIAL" shell getprop ro.product.cpu.abi | tr -d '\r')" = x86_64
ANDROID_SERIAL="$MIGO_ARM64_SERIAL" bash scripts/gen-snapshot.sh arm64 --product-profile full --snapshot-kind host
ANDROID_SERIAL="$MIGO_ARM64_SERIAL" bash scripts/gen-snapshot.sh arm64 --product-profile slim --snapshot-kind host
ANDROID_SERIAL="$MIGO_ARM64_SERIAL" bash scripts/gen-snapshot.sh arm64 --product-profile full --snapshot-kind worker
ANDROID_SERIAL="$MIGO_X86_64_SERIAL" bash scripts/gen-snapshot.sh x86_64 --product-profile full --snapshot-kind host
ANDROID_SERIAL="$MIGO_X86_64_SERIAL" bash scripts/gen-snapshot.sh x86_64 --product-profile slim --snapshot-kind host
ANDROID_SERIAL="$MIGO_X86_64_SERIAL" bash scripts/gen-snapshot.sh x86_64 --product-profile full --snapshot-kind worker
```

- [ ] **Step 3: Verify fresh fingerprints and runtime tests**

```bash
bash scripts/check-snapshot-freshness.sh --snapshot-kind host --product-profile full aarch64 x86_64
bash scripts/check-snapshot-freshness.sh --snapshot-kind host --product-profile slim aarch64 x86_64
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full aarch64 x86_64
cd engine
bash ../scripts/run-cargo-lib-test-filter.sh migo-runtime-v8 snapshot_fingerprint
cargo test -p migo-runtime-v8 --lib --locked --offline
cargo test -p migo-core --lib --locked --offline
cargo test -p migo-audio --lib --locked --offline
cargo test -p migo-graphics --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cargo test -p migo-capi --lib --locked --offline
cargo check --workspace --all-targets --locked --offline
```

Expected: PASS; all six manifests match current Rust, JS, feature, Deno, V8,
profile, kind, and ABI identities.

- [ ] **Step 4: Run Android profile and contract regressions**

```bash
cd platforms/android
./gradlew :library:testFullDebugUnitTest
./gradlew :library:testSlimDebugUnitTest
./gradlew :library:lintDebug
cd ../..
bash scripts/test-runtime-restart-generation-contract.sh
bash scripts/test-platform-v8-boundary-contract.sh
bash scripts/test-product-profiles.sh
bash scripts/test-android-sdk-contract.sh
bash scripts/test-r9-worker-snapshot.sh
```

Expected: PASS; each Gradle filter executes at least one test and both profiles
retain their exact API surface.

- [ ] **Step 5: Commit snapshot bytes and final verified state**

```bash
git add engine/crates/runtime-v8/snapshots/SNAPSHOT-full-aarch64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-full-aarch64.bin.manifest.json \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-full-x86_64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-full-x86_64.bin.manifest.json \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-aarch64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-aarch64.bin.manifest.json \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-x86_64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-slim-x86_64.bin.manifest.json \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-aarch64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-aarch64.bin.manifest.json \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin \
  engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin.manifest.json \
  engine/crates/runtime-v8/src/tests/snapshot_fingerprint.rs
git commit -m "build: refresh runtime restart snapshots"
```

## Explicit Follow-On Boundary

The current Linux and Windows `PlatformServices` and `CapiHostKit` expose no
asynchronous result, resource, or event-stream callback source, so their
restart-fence implementations are deliberately no-ops and are covered by tests
in Task 10. Adding any such capability is a separate feature: its implementation
plan must add a generation token or Host ID to that callback source and replace
the no-op fence before enabling the capability. That future work is not hidden
inside this plan and is not a reason to weaken the Android or core boundary.

## Completion Evidence

The work is complete only when:

- every allocator, registry, Android transport, fence, audio, render, race, and
  snapshot command above passes from the recorded revision;
- the static audit finds no local cross-runtime counter or ID-less FIFO result
  fallback;
- a test proves read-only preflight failure retains the old isolate;
- tests prove commit failure and every failure after revocation begins publish no new
  isolate and terminates the Host normally;
- Android tests prove restart preserves public handler registrations and
  standing permission decisions while destroying runtime-owned resources; and
- all six Android snapshot artifacts and manifests are regenerated on their
  matching ABI and pass freshness validation.
