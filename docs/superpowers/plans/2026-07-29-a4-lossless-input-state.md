# Lossless Input State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep high-rate input bounded and allocation-free in steady state
while guaranteeing that accepted touch, pointer, key, composition, gamepad, and
focus-loss transitions converge to the correct final state.

**Architecture:** Replace the semaphore in front of Tokio's unbounded host
queue with one preallocated ordered queue that has a normal input budget and a
separate reliable-transition reserve. Producers explicitly identify
coalescible streams and reliable transitions; the queue replaces only the
latest eligible state in that stream and a reliable transition removes older
coalescible state before consuming reserve. Host-side input state observation
generates focus-loss retractions at the single ordered consumer, while C and
Android adapters report actual acceptance and observable saturation.

**Tech Stack:** Rust 2024, `parking_lot`, Tokio `Notify`, fixed payload pools,
JNI registered natives, Java direct `ByteBuffer`, C ABI callbacks, wire-format
metrics.

---

## Invariants

- Normal and reliable input occupy at most
  `HOST_NORMAL_COMMAND_CAPACITY + HOST_RELIABLE_INPUT_RESERVE` queue entries.
- The input send path never waits and the numeric touch, pointer, and gamepad
  steady paths do not allocate after Host construction.
- A coalescible command replaces an older command only when that command is
  still the newest command for the same logical stream. It never crosses a
  transition in that stream.
- Touch `START`/`END`/`CANCEL`, pointer `DOWN`/`UP`, physical key `UP`, keyboard
  `COMPLETE`, composition `START`/`END`, and gamepad connect/disconnect use the
  reliable reserve. A terminal command first supersedes obsolete coalescible
  state for its stream.
- Returning `Ok(Enqueued | Coalesced | Reserved)` means the event will be
  observed or was safely replaced by newer equivalent state. Returning `Full`
  means it was not accepted and is reported to metrics and the host.
- Critical lifecycle and Surface commands retain their existing ordered
  control capability. They share the same FIFO linearization point but are not
  charged to the untrusted input budgets.
- Focus loss is ordered after all earlier accepted input. The Host retracts
  active touches, pointer buttons, physical keys, and composition exactly once
  before it dispatches the focus-loss callback.
- Android `dispatchTouchEvent` returns true only for supported events accepted
  or safely coalesced by native code.

## File Responsibility Map

- `engine/crates/shared/src/host_channel.rs`: one ordered queue, input stream
  metadata, coalescing, reliable reserve, closure/wakeup semantics.
- `engine/crates/core/src/runtime/registry.rs`: semantic `HostIngress` methods,
  fixed payload pools, and per-session transport metrics.
- `engine/crates/core/src/runtime/input_state.rs`: allocation-free motion state
  observation and deterministic focus-loss retractions.
- `engine/crates/core/src/runtime/host.rs`: observe accepted commands at the
  sole consumer and dispatch retractions before blur.
- `engine/crates/shared/src/stats.rs`: v6 wire metrics for coalescing, reserve
  use, and actual saturation.
- `engine/crates/capi/src/{input,keyboard,gamepad,lib}.rs`: classify C input,
  return `WOULD_BLOCK` only for actual refusal, and rate-limit error callbacks
  by saturation episode.
- `engine/crates/platform/src/android/jni/{inbound,profile_contract}.rs`:
  classify touch, return JNI boolean, and report saturation once per episode.
- `platforms/android/library/src/main/java/com/migo/runtime/internal/`:
  propagate the native touch result without allocating on the hot path.
- `platforms/android/library/src/main/java/com/migo/runtime/`:
  expose the result from `GameSession` and parse the v6 metrics tail.
- `include/migo/input.h` and `include/migo/session.h`: document acceptance,
  coalescing, focus-loss convergence, and retry behavior.
- `scripts/test-input-transport-contract.sh`: fail-closed source and workflow
  integration contract.

## Task 1: Build One Ordered, Preallocated Input Queue

**Files:**
- Modify: `engine/crates/shared/src/host_channel.rs`

- [x] **Step 1: Add failing queue tests**

Add tests covering these exact cases:

```rust
#[test]
fn move_replaces_only_latest_state_before_same_stream_transition() {
    let (tx, _, mut rx) = channel_with_reserve(3, 1);
    assert_eq!(
        tx.try_send_coalescible(InputStream::Pointer, mouse_move(1.0)),
        Ok(InputSendOutcome::Enqueued)
    );
    assert_eq!(
        tx.try_send_coalescible(InputStream::Pointer, mouse_move(2.0)),
        Ok(InputSendOutcome::Coalesced)
    );
    tx.try_send_reliable(Some(InputStream::Pointer), mouse_down()).unwrap();
    tx.try_send_coalescible(InputStream::Pointer, mouse_move(3.0)).unwrap();

    assert_move_x(rx.try_recv().unwrap(), 2.0);
    assert!(matches!(rx.try_recv(), Ok(HostCommand::OnMouseDown { .. })));
    assert_move_x(rx.try_recv().unwrap(), 3.0);
}

#[test]
fn terminal_supersedes_motion_and_uses_reserved_capacity() {
    let (tx, _, mut rx) = channel_with_reserve(2, 1);
    tx.try_send_coalescible(InputStream::Pointer, mouse_move(1.0)).unwrap();
    tx.try_send(HostCommand::Restart).unwrap();

    assert_eq!(
        tx.try_send_terminal(Some(InputStream::Pointer), mouse_up()),
        Ok(InputSendOutcome::Enqueued)
    );
    assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
    assert!(matches!(rx.try_recv(), Ok(HostCommand::OnMouseUp { .. })));
}

#[test]
fn reliable_transition_uses_reserve_when_normal_lane_is_full() {
    let (tx, _, mut rx) = channel_with_reserve(1, 1);
    tx.try_send(HostCommand::Restart).unwrap();
    assert_eq!(
        tx.try_send_reliable(None, key_up("A")),
        Ok(InputSendOutcome::Reserved)
    );
    assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
    assert!(matches!(rx.try_recv(), Ok(HostCommand::OnKeyUp { .. })));
}

#[test]
fn reliable_reserve_is_bounded_and_returns_original_command() {
    let (tx, _, _rx) = channel_with_reserve(1, 1);
    tx.try_send(HostCommand::Restart).unwrap();
    tx.try_send_reliable(None, key_up("A")).unwrap();
    assert!(matches!(
        tx.try_send_reliable(None, key_up("B")),
        Err(TrySendError::Full(HostCommand::OnKeyUp { .. }))
    ));
}
```

Also retain and adapt the existing FIFO, normal-budget release,
critical-bypass, receiver-close, and async-receive tests. Add a multi-producer
barrier test that assigns commands on either side of a reliable transition and
asserts the queue's mutex linearization order.

Run:

```bash
cd engine
cargo test -p migo-shared host_channel::tests --lib --locked --offline
```

Expected: FAIL because stream classification, reliable reserve, and
coalescing do not exist.

- [x] **Step 2: Implement the transport types**

Define the public input contract:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputStream {
    Touch,
    Pointer,
    Composition,
    Gamepad(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSendOutcome {
    Enqueued,
    Coalesced,
    Reserved,
}

impl HostCommandSender {
    pub fn try_send_coalescible(
        &self,
        stream: InputStream,
        command: HostCommand,
    ) -> Result<InputSendOutcome, TrySendError<HostCommand>>;

    pub fn try_send_reliable(
        &self,
        stream: Option<InputStream>,
        command: HostCommand,
    ) -> Result<InputSendOutcome, TrySendError<HostCommand>>;

    pub fn try_send_terminal(
        &self,
        stream: Option<InputStream>,
        command: HostCommand,
    ) -> Result<InputSendOutcome, TrySendError<HostCommand>>;
}
```

Back the channel with:

```rust
struct QueueState {
    entries: VecDeque<QueuedHostCommand>,
    normal_len: usize,
    reserved_len: usize,
    receiver_open: bool,
    normal_senders: usize,
    critical_senders: usize,
}

struct SharedQueue {
    state: parking_lot::Mutex<QueueState>,
    not_empty: tokio::sync::Notify,
    normal_capacity: usize,
    reliable_capacity: usize,
}
```

Preallocate `VecDeque` for the two input budgets plus a small documented
control-plane headroom. Implement `Clone`/`Drop` for both sender capabilities
so async receive drains already queued commands and then returns `None` after
the last sender closes. `HostCommandReceiver::drop` marks the receiver closed
and drops every queued payload immediately.

In `recv`, create and enable a `Notify::notified()` future before checking the
queue, then recheck under the mutex before awaiting. This prevents a send
between the empty check and the await from becoming a lost wakeup.

For coalescing, scan backward only to the newest entry with the same stream. If
it is coalescible, replace its command in place and preserve its queue
position/lane. If it is a transition or absent, enqueue normally.

For reliable sends, preserve earlier state and use normal capacity if available
and reliable capacity otherwise. For terminal sends, first prove capacity, then
remove every older coalescible entry for the same stream and update its lane
count. Return `Full(command)` without mutating accepted state when both lanes
are exhausted.

Keep `channel(normal_capacity)` as the production constructor and add a private
or test-visible `channel_with_reserve(normal_capacity, reliable_capacity)`.

- [x] **Step 3: Verify wakeup, order, bounds, and allocation behavior**

Run:

```bash
cd engine
cargo test -p migo-shared host_channel::tests --lib --locked --offline
cargo test -p migo-shared host_channel --doc --locked --offline
```

Expected: PASS. The queue never exceeds its two input counts, the receiver
cannot miss a wakeup, and rejected sends return ownership of the original
command.

## Task 2: Add Semantic Ingress And Observable Metrics

**Files:**
- Modify: `engine/crates/core/src/runtime/registry.rs`
- Modify: `engine/crates/shared/src/stats.rs`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/GameSession.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/PerformanceSnapshot.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/DebugOverlayView.java`

- [x] **Step 1: Add failing HostIngress saturation tests**

Replace the old "second touch is full" expectation with tests that:

- fill the normal budget and coalesce repeated touch moves into one slot;
- enqueue touch end/cancel through the reliable reserve;
- coalesce gamepad state independently by index;
- preserve pointer down/move/up order;
- count one coalescing event, one reserve use, and only actual refusals as
  saturation;
- exhaust both normal and reliable capacities and prove the rejected terminal
  command is returned as `Full`.

Run:

```bash
cd engine
cargo test -p migo-core runtime::registry::tests --lib --locked --offline
```

Expected: FAIL because `HostIngress` has only undifferentiated `try_send`.

- [x] **Step 2: Expose semantic nonblocking methods**

Set:

```rust
pub(crate) const HOST_NORMAL_COMMAND_CAPACITY: usize = 512;
pub(crate) const HOST_RELIABLE_INPUT_RESERVE: usize = 64;
const HOST_PAYLOAD_POOL_CAPACITY: usize =
    HOST_NORMAL_COMMAND_CAPACITY + HOST_RELIABLE_INPUT_RESERVE + 1;
```

Add:

```rust
pub fn try_send_touch(&self, touch: TouchData)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_pointer(&self, command: HostCommand)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_key(&self, command: HostCommand)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_keyboard(&self, command: HostCommand)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_composition(&self, command: HostCommand)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_gamepad_connection(&self, command: HostCommand)
    -> Result<InputSendOutcome, HostIngressSendError>;
pub fn try_send_gamepad_state(&self, state: GamepadState)
    -> Result<InputSendOutcome, HostIngressSendError>;
```

Classify touch move, pointer move, composition update, and each gamepad state
index as coalescible. Classify touch start/end/cancel, pointer down/up, key up,
keyboard complete, composition start/end, and gamepad connect/disconnect as
reliable. Keep wheel and key down as normal; repeated key-down may return
backpressure, while its matching key-up still has reserve.

Record `Coalesced`, `Reserved`, and `Full` centrally in `HostIngress`, so no
adapter can double count. Acquiring a pooled touch/gamepad payload before
coalescing is safe: replacement drops the old pooled payload back into the
fixed pool.

- [x] **Step 3: Append v6 transport metrics**

Append these `u32` fields without moving v1-v5 offsets:

```rust
pub input_coalesced: u32,
pub input_reliable_reserve_uses: u32,
pub input_saturation_events: u32,
```

Set `RenderMetricsSnapshot::VERSION` to `6`, `PAYLOAD_LEN` to `140`, and
`BYTE_LEN` to `144`. Add matching atomics to `DebugStats`, serialize at offsets
132, 136, and 140, and update exact byte-layout tests.

Update Android parsing defensively by packet version/length. Preserve the
existing `PerformanceSnapshot` constructor as a source-compatible overload and
add the three v6 fields in a new full constructor. Show saturation and
coalescing only in the existing debug diagnostics area, not as per-frame UI
work.

- [x] **Step 4: Verify wire compatibility and hot-path bounds**

Run:

```bash
cd engine
cargo test -p migo-shared stats::tests --lib --locked --offline
cargo test -p migo-core runtime::registry::tests --lib --locked --offline
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest
```

Expected: PASS. Old offsets are unchanged; v6 readers expose all three fields;
payload and queue allocations are fixed at Host construction.

## Task 3: Retract Active Input Before Focus Loss

**Files:**
- Create: `engine/crates/core/src/runtime/input_state.rs`
- Modify: `engine/crates/core/src/runtime/mod.rs`
- Modify: `engine/crates/core/src/runtime/host.rs`

- [x] **Step 1: Add failing pure state-machine tests**

Create tests that feed `InputState` accepted commands and use a callback to
collect focus retractions:

```rust
#[test]
fn focus_loss_retracts_every_active_stream_once() {
    let mut state = InputState::default();
    state.observe(&touch_start());
    state.observe(&touch_move());
    state.observe(&mouse_down());
    state.observe(&key_down("KeyA"));
    state.observe(&composition_start("preedit"));

    let mut got = Vec::new();
    state.retract_for_focus_loss(|event| got.push(event));

    assert_touch_cancel(&got);
    assert_mouse_up(&got, 0);
    assert_key_up(&got, "KeyA");
    assert_composition_end(&got, "");

    let mut second = Vec::new();
    state.retract_for_focus_loss(|event| second.push(event));
    assert!(second.is_empty());
}
```

Add cases for partial multi-touch end, pointer move coordinates, repeated key
down, normal key up, composition end, and an input sequence after refocus.

Run:

```bash
cd engine
cargo test -p migo-core runtime::input_state::tests --lib --locked --offline
```

Expected: FAIL because no Host input state exists.

- [x] **Step 2: Implement fixed motion state and cold transition state**

Use fixed `[Option<TouchPoint>; 10]` and fixed pointer-button slots for
touch/pointer motion. Key down already owns allocated strings; move those
strings into a keyed map and drain them on focus loss. Composition needs only
an activity bit because the synthetic end payload is intentionally empty.
Specialized observers consume owned key strings without cloning and keep the
motion path allocation-free. Define:

```rust
pub(crate) enum InputRetraction {
    TouchCancel(TouchData),
    MouseUp { x: f32, y: f32, button: u32, timestamp_ms: f64 },
    KeyUp {
        key: String,
        code: String,
        timestamp_ms: f64,
        modifiers: u32,
    },
    CompositionEnd,
}

impl InputState {
    pub(crate) fn observe_touch(&mut self, touch: &TouchData);
    pub(crate) fn observe_mouse_down(...);
    pub(crate) fn observe_mouse_move(...);
    pub(crate) fn observe_mouse_up(...);
    pub(crate) fn observe_key_down(...);
    pub(crate) fn observe_key_up(...);
    pub(crate) fn observe_composition_start(&mut self);
    pub(crate) fn observe_composition_end(&mut self);
    pub(crate) fn retract_for_focus_loss(
        &mut self,
        dispatch: impl FnMut(InputRetraction),
    );
}
```

Do not clone motion payloads or allocate while observing touch/pointer moves.
Use the last accepted event timestamp for synthetic pointer/key release. Touch
cancel includes every still-active point with changed+removed flags.

- [x] **Step 3: Integrate at the single Host consumer**

Observe each input command immediately before its existing JS dispatch. For
`OnFocusChanged { focused: false }`, drain `InputState` and call the existing
touch, mouse-up, key-up, and composition-end bindings before
`dispatch_focus_changed(false)`. Do not enqueue synthetic commands and do not
send them back through a saturated producer queue.

Run:

```bash
cd engine
cargo test -p migo-core runtime::input_state::tests --lib --locked --offline
cargo check -p migo-core --all-targets --locked --offline
```

Expected: PASS, with focus loss sharing the same FIFO consumer order as all
accepted input.

## Task 4: Classify C ABI Input And Report Saturation

**Files:**
- Modify: `engine/crates/capi/src/lib.rs`
- Modify: `engine/crates/capi/src/input.rs`
- Modify: `engine/crates/capi/src/keyboard.rs`
- Modify: `engine/crates/capi/src/gamepad.rs`
- Modify: `engine/crates/capi/src/test_support.rs`
- Modify: `include/migo/input.h`
- Modify: `include/migo/session.h`

- [x] **Step 1: Add failing capacity and callback tests**

Exercise a one-slot `HostIngress` directly for every semantic class, then
verify each C entry point's validation independently and pin adapter routing in
the static contract. This avoids exporting a test-only ingress-injection seam
from `migo-core`. Prove:

- touch move/pointer move/composition update/gamepad state return `MIGO_OK` when
  coalesced;
- touch end/cancel, pointer up, key up, keyboard complete, composition end, and
  gamepad disconnect return `MIGO_OK` when the normal lane is full;
- one additional reliable transition returns `MIGO_ERROR_WOULD_BLOCK` when its
  reserve is also full;
- a failed gamepad connect/disconnect rolls its topology reservation back;
- one saturation episode posts one `on_error(MIGO_ERROR_WOULD_BLOCK, ...)`
  callback; a successful accepted/coalesced event rearms reporting.

Run:

```bash
cd engine
cargo test -p migo-core runtime::registry::tests --lib --locked --offline
cargo test -p migo-capi input_saturation_callback_is_once_per_episode --lib --locked --offline
```

Expected: FAIL because all C input calls use normal `try_send`.

- [x] **Step 2: Route each ABI through semantic ingress**

Change `map_ingress_result` to accept the pinned `MigoSession` and
`InputSendOutcome`. Treat every successful outcome as `MIGO_OK`. On `Full`,
return `MIGO_ERROR_WOULD_BLOCK`; on `Closed`, return
`MIGO_ERROR_INVALID_STATE`.

Add one `AtomicBool` saturation-episode gate to `MigoSession`. On the first
`Full`, clone the installed notifier outside the Session lock and post a
recoverable error containing the entry-point name and bounded-capacity detail.
Clear the gate after the next accepted or coalesced input. Never invoke a host
callback while holding the Session mutex.

Use the semantic ingress methods from Task 2. Commit gamepad topology only
after an accepted connection transition. Keep validation errors distinct from
queue pressure.

- [x] **Step 3: Document exact observable behavior**

Document in public headers:

- `MIGO_OK` means enqueued or safely coalesced;
- `MIGO_ERROR_WOULD_BLOCK` means not accepted and may be retried;
- transitions use reserve but reserve exhaustion is still possible under a
  producer that ignores backpressure;
- hosts must deliver focus changes, and Migo retracts accepted active input
  before the focus-loss callback.

Run:

```bash
cd engine
cargo test -p migo-capi input_saturation_callback_is_once_per_episode --lib --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh --core
```

Expected: PASS.

## Task 5: Propagate Real Android Touch Acceptance

**Files:**
- Modify: `engine/crates/platform/src/android/jni/inbound.rs`
- Modify: `engine/crates/platform/src/android/jni/profile_contract.rs`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeBridge.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeMethods.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/TouchEventHandler.java`
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/GameSession.java`
- Test: `platforms/android/library/src/test/java/com/migo/runtime/internal/TouchEventHandlerTest.java`

- [x] **Step 1: Add failing Java and JNI contract tests**

Add Java tests using an injectable package-private native touch sink so
`TouchEventHandler.dispatch` proves:

- supported accepted/coalesced events return true;
- native refusal returns false;
- invalid session, null event, unsupported action, unrepresentable changed
  pointer, or empty packed input returns false;
- the same direct buffer instance is reused across moves.

Change the registered-native profile expectation to:

```rust
("onTouchEvent", "(IIJILjava/nio/ByteBuffer;)Z")
```

Run:

```bash
cd engine
cargo test -p migo-platform android::jni::profile_contract --lib --locked --offline
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest --tests '*TouchEventHandlerTest'
```

Expected: FAIL while the native method returns `void`.

- [x] **Step 2: Return JNI boolean end to end**

Make Rust `onTouch` return `jboolean`. Return `JNI_TRUE` only for
`InputSendOutcome::{Enqueued, Coalesced, Reserved}`. Return `JNI_FALSE` for
invalid buffer/count/action, missing/closed ingress, or real saturation.

On the first saturation episode, call the existing Android `onError` outbound
bridge with a recoverable input-saturation code and detail; rearm after the next
successful touch. Do not call Java on every move and do not allocate on the
successful path.

Change `NativeBridge.onTouchEvent`, `NativeMethods.onTouchRaw`,
`TouchEventHandler.dispatch`, and `GameSession.dispatchTouchEvent` to return
boolean. The deprecated array bridge returns the same native result. Preserve
main-thread confinement and the reusable direct buffer.

- [x] **Step 3: Verify JNI shape and Android behavior**

Run:

```bash
cd engine
cargo test -p migo-platform android::jni --lib --locked --offline
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest
./gradlew :library:lintFullDebug
```

Expected: PASS with a byte-for-byte matching `Z` native signature and no
unconditional handled return.

## Task 6: Add Fail-Closed Delivery Gates

**Files:**
- Create: `scripts/test-input-transport-contract.sh`
- Modify: `.github/workflows/pr-ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md`
- Modify: `docs/superpowers/plans/2026-07-29-three-platform-delivery.md`

- [x] **Step 1: Add the static delivery contract**

The script must fail unless:

- the host channel exposes coalescible and reliable send methods;
- touch/pointer/composition/gamepad adapters use their semantic ingress;
- the reliable reserve constant is non-zero and payload pools cover it;
- focus loss drains Host input state before the focus callback;
- JNI declares `onTouchEvent` with `Z` in Rust and Java;
- `GameSession.dispatchTouchEvent` returns the handler result;
- v6 metrics contain coalesced, reserve, and saturation counters;
- both PR and release workflows execute this script.

Run:

```bash
bash scripts/test-input-transport-contract.sh
```

Expected: FAIL before workflow wiring, then PASS.

- [x] **Step 2: Run capacity and convergence verification**

Run:

```bash
cd engine
cargo fmt --all -- --check
cargo test -p migo-shared host_channel::tests --lib --locked --offline
cargo test -p migo-shared stats::tests --lib --locked --offline
cargo test -p migo-core runtime::input_state::tests --lib --locked --offline
cargo test -p migo-core runtime::registry::tests --lib --locked --offline
cargo test -p migo-capi input_saturation_callback_is_once_per_episode --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cargo check -p migo-shared -p migo-core -p migo-capi -p migo-platform \
  --all-targets --locked --offline
cd ../platforms/android
./gradlew :library:testFullDebugUnitTest
./gradlew :library:lintFullDebug
cd ../..
bash scripts/test-input-transport-contract.sh
bash scripts/test-surface-attachment-contract.sh
git diff --check
```

Expected: PASS. Tests explicitly fill normal lanes, every reliable reserve,
and pooled mailboxes before checking final state.

Verification evidence recorded on 2026-07-29:

- `cargo fmt --all -- --check`: PASS.
- `cargo test -p migo-shared -p migo-core -p migo-capi -p migo-platform
  --lib --locked --offline`: PASS, including the bounded queue, metrics,
  focus-retraction, saturation callback, and JNI profile tests.
- `cargo test --doc -p migo-shared --locked --offline`: PASS, 2 compile-fail
  contract tests.
- `cargo check -p migo-shared -p migo-core -p migo-capi -p migo-platform
  --all-targets --locked --offline`: PASS.
- `:library:testFullDebugUnitTest` and `:library:testSlimDebugUnitTest`: PASS.
- `:library:lintFullDebug` and `:library:lintSlimDebug`: PASS without a
  baseline.
- `scripts/test-input-transport-contract.sh`,
  `scripts/test-surface-attachment-contract.sh`, and
  `scripts/test-c-abi-surface-candidate.sh --core`: PASS.
- `git diff --check`: PASS.

- [x] **Step 3: Update the delivery ledgers and commit**

Record factual command evidence in this plan and the umbrella plan. Do not mark
A4 complete if linked Rust tests are blocked by missing native development
libraries or Android verification was not run.

Commit only verified files:

```bash
git add \
  engine/crates/shared/src/host_channel.rs \
  engine/crates/shared/src/stats.rs \
  engine/crates/core/src/runtime \
  engine/crates/capi/src \
  engine/crates/platform/src/android/jni \
  platforms/android/library/src/main/java \
  platforms/android/library/src/test \
  include/migo/input.h include/migo/session.h \
  scripts/test-input-transport-contract.sh \
  .github/workflows/pr-ci.yml .github/workflows/release.yml \
  docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md \
  docs/superpowers/plans/2026-07-29-three-platform-delivery.md \
  docs/superpowers/plans/2026-07-29-a4-lossless-input-state.md
git commit -m "fix: make bounded input state converge"
```

Do not push, tag, publish, or create a release.
