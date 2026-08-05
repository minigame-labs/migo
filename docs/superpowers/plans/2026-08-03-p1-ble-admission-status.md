## P1 BLE Callback Admission — Status After Resumption (2026-08-03)

**Branch:** `delivery/three-platform-rc`
**Commits:** `2433e16` (inherited squashed WIP) → `cb39464` (this work, local only)
**Safety stash:** `stash@{0} phase-a-before-master-d41389d` — preserved, untouched.
**Not done:** no push, no tag, no release, no publication.

### Starting position

The inherited tree contained an unverified, partially edited fix for a
connection-state race: the connected-state callback had been moved inside the
`scope.bluetooth` admission and `GattAttempt.dispatchIfActive`, and three
concurrency fixtures had been added, but no RED, GREEN, compilation, mutation,
or review evidence had been reported. The prior 23/23 focused result and spec
approval predated those edits and were treated as stale.

### What was completed

**1. The interrupted change was verified, not assumed.** The inherited
production code compiles and its focused tests pass. Establishing that took
correcting the test invocation itself: `--tests` filters bind only to the task
they follow on the Gradle command line, so the previously used single-filter
form silently left one profile unfiltered. Per-task filters are now used.

**2. Two surviving mutants were found and killed.** Mutation testing showed the
inherited fixtures did not constrain two of the invariants they claimed:

| Mutation | Inherited suite | Now |
| --- | --- | --- |
| `dispatchIfActive` ignores `acceptingCallbacks` | survived — no test failed | killed (`connectedCallbackAfterRetainedCloseFailure…`) |
| connected path bypasses the standing-scope admission | killed by 3 tests | killed by 3 tests |
| `beginClose` not synchronised with attempt admission | survived — the fixture won a race | killed (`connectedCallbackReportsBeforeConcurrentClose…`) |
| `awaitIdle` never waits | 50/50 coin flip on 3 fixtures | killed deterministically by all 3 |

The first mutant was reachable because a failing `gatt.close()` deliberately
retains the map entry for retry, so map identity still matches while the attempt
is retired — only `acceptingCallbacks` rejects the callback in that window. A
RED-first cycle was run for it: mutation applied, new test written and observed
failing for the intended reason (`discoverServices()` invoked on a retired
GATT), mutation reverted, test green.

**3. Two real defects found by independent code-quality review were fixed.**

- *A superseded attempt corrupted live connection state.*
  `connectionStateReporter.report(deviceId, false)` is keyed only by device
  address. A late callback from a replaced attempt therefore reported the
  **live replacement** connection as disconnected. Fixed by reporting a
  not-connected transition only when no other attempt owns that device.
- *Ownership did not follow `close()`.* `ResourceCleanup.runAll(disconnect,
  close)` rethrew a disconnect failure even when `close()` had already released
  the handle, skipping removal and leaving a closed GATT mapped as live until a
  retry closed it a second time. Ownership now follows `close()`; a
  disconnect-only failure is reported through the cleanup-failure path instead
  of blocking ownership transfer.

Both fixes have regression tests that were confirmed to fail without the fix
(`[AB:CD:true, AB:CD:false]` vs `[AB:CD:true]`, and an escaping
`gatt disconnect failed`).

**4. Concurrency fixtures were made to prove their invariants.** Teardown
blocked-ness, lease drain before native permission update, and lease drain
before cancellation are now asserted as bounded negative observations gated on
a start latch; GATT close is additionally checked against in-flight connection
dispatch; every unbounded `join()` is now bounded with a liveness assertion.
The bounded negative observation cannot produce a false failure for correct
code, because correct linearisation can never release the latch.

**5. Lock ordering was audited across the Java/Rust boundary.** Order is
gate session-map monitor → per-session monitor (released before the callback
body) → `GattAttempt` monitor → Rust `host_senders` read lock. No holder of the
attempt monitor ever waits for a gate monitor: `runIfGranted` re-acquires the
session monitor only after the callback body has returned, and every teardown
path runs `awaitIdle` before any cancellation. `send_command_to_host` clones the
sender under a read lock and uses `try_send`, so the JNI report cannot block
while the attempt monitor and permission lease are held. `TerminalCloseQueue`
is lock-free and only posts to a Handler. No cycle was found.

### Evidence

| Suite | Result |
| --- | --- |
| Android Full JVM unit suite | 86 tests, 0 failures / errors / skips |
| Android Slim JVM unit suite | 86 tests, 0 failures / errors / skips |
| Focused gate + GATT (per profile) | 12 + 17 |
| Rust `migo-shared` / `core` / `platform` / `capi` | 596 passed, 1 ignored (X11 native) |
| `scripts/test-permission-coverage-contract.sh` | 30 gated, 8 cleanup, 38 sensitive |
| `git diff --check` | clean |

Review status: the independent spec-compliance review approved the state that
preceded the two fixes above, raising one important test-determinism finding
(now fixed) and three minor findings (below). The independent code-quality
review did not approve that state; its two high findings are the defects fixed
in this commit.

### What remains

**Re-review before this stage can be called approved.** The two high-severity
fixes and the fixture changes have not yet been through a repeat spec review or
a repeat code-quality review. That is the immediate next step.

**Open code-quality findings, not yet addressed (all medium):**

1. `PermissionOperationGate.cancelAll` runs externally supplied cancellation
   actions while `update`, `revoke`, and `close` hold the per-session monitor,
   convoying unrelated scopes behind Binder/JNI work. Proposed fix: snapshot and
   mark cancellations under the session monitor, execute them after releasing it
   while still holding the transition lock, then re-acquire to remove successes
   and retain failures for retry. No current reverse-lock edge exists; the
   boundary is unsafe for future cleanup actions.
2. BLE notification hot path: `PermissionOperationGate.session()` takes a
   monitor on a `HashMap` shared by every session for each notification, and
   each callback allocates an `AndroidGattConnection` plus capturing lambdas.
   Proposed fix: `ConcurrentHashMap` for lock-free lookup with a narrow lock
   retained only for monotonic `open`; intern one `GattConnection` per attempt;
   replace callback-under-monitor dispatch with explicit counted begin/end
   admission so foreign calls run outside the attempt monitor. Needs a
   throughput/allocation regression test, which does not exist.
3. Remaining fixture hardening: releases are not all inside `finally`, and the
   bounded negative observation could in principle let a mutant pass if the
   teardown thread is descheduled for the whole budget. A positive
   blocked-state observation would close that gap.

**Open spec-review minor findings:**

4. `GattAttempt.matches` accepts by raw handle when two wrappers share one
   `BluetoothGatt`, which is the production path (each callback wraps `gatt`
   afresh), but no test exercises the accept side because the fake returns
   `null`. Covering it needs either a mocking dependency on the library test
   classpath or an identity type that does not require a real `BluetoothGatt`.
   A regression here fails closed, so it is not a leak risk.
5. `getBLEMTU` has no `requireConnectPermission` check unlike every sibling
   operation. It only reads a cache and the runtime layer gates the op on
   `Scope::Bluetooth`, so nothing escapes a denied scope, but the local
   discipline is inconsistent.
6. The production admission wiring (`NativeExports.runIfPermissionGranted`
   delegation) is never exercised by a test; all fixtures inject their own
   admission.

**Previously deferred P2 items, still deferred:**

7. When cleanup of a rejected late GATT candidate throws, the only reference is
   discarded, so terminal cleanup cannot retry it.
8. `CameraSlot` publishes ownership before `mgr.create`, so terminal cleanup can
   race initialisation; creation must become transactional.
9. Generated Full and Slim merged-manifest verification at API 26, 28, and 31
   against real shipping AARs.

**Then the ledger continues unchanged:** A1–A3, A5, A6, A8–A13 (including the
12-task runtime restart generation boundary plan), followed by Phases B–E. The
real consumer R8 gate stays blocked until genuine Full/Slim multi-ABI AARs
exist; the repository still lacks the Android V8 archive at
`engine/third_party/rusty_v8/aarch64/librusty_v8.a`, which must come through the
verified component flow rather than a host archive.

Not run in this session, and therefore not claimed: Android lint for both
profiles, the surface/input/X11/owned-host contract scripts, and
`cargo fmt --all --check`.
