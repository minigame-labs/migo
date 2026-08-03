# Three-Platform Delivery Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resume the approved Migo delivery effort from the local checkpoint and produce a locally verified `0.10.0-rc.1` release candidate for Android, Linux, and Windows without pushing, tagging, or publishing it.

**Architecture:** The design spec and delivery ledger are the authority. Finish runtime and platform correctness first, then hermetic packaging, native qualification, examples, performance evidence, and public product documentation; every phase remains fail-closed and is committed only after behavioral verification plus independent spec and quality review.

**Tech Stack:** Rust 2024, V8/Deno Core, Skia/EGL/ANGLE, JNI/Java/Gradle, Android NDK, C/C++ ABI, X11/Wayland, Win32/MSVC, CMake/Ninja, Bash/PowerShell, GitHub Actions, SBOM and provenance tooling.

---

## Authority And Read Order

Read these files before changing code:

1. `docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md`
2. `docs/superpowers/plans/2026-07-29-three-platform-delivery.md`
3. `docs/superpowers/plans/2026-08-02-runtime-restart-generation-boundary.md`
4. The A1-A4 detailed plans referenced by the delivery ledger.
5. This handoff document.

The target worktree is
`/data/work/opensource/migo/.worktrees/three-platform-delivery` on branch
`delivery/three-platform-rc`. The adjacent examples repository is
`/data/work/opensource/migo-examples`.

## Non-Negotiable Constraints

- Follow TDD for every behavior change: observe the focused test fail for the
  intended reason, implement the smallest correct design, then run focused and
  regression suites.
- Run an independent spec-compliance review and then an independent code-quality
  review for every implementation task. Do not advance with an open Critical or
  Important finding.
- Use no fake archives, empty native libraries, host artifacts for another
  target, success-masking shell logic, timeout escape from owned shutdown, or
  mutable/unidentified release input.
- Commit verified stages locally. Never push, tag, publish a release, or delete
  the existing `phase-a-before-master-d41389d` stash.
- Keep Business Source License 1.1 and describe the current product as
  source-available, not OSI open source.
- Do not weaken required Android, Linux, Windows, Full, Slim, ABI, manifest,
  report, sample-count, or artifact gates into optional skips.
- Work with existing user changes. Do not reset, restore, or rewrite unrelated
  work to make the tree look clean.
- If a real SDK/tool installation is the only blocker, stop and give the user
  the exact install command. Missing target artifacts are not permission to
  substitute incompatible bytes.

## Current Stop State (2026-08-02)

The user explicitly stopped the current session while the connection-state
quality finding was being fixed. This is an **uncommitted, partially edited,
and unverified WIP**, not a delivery checkpoint.

- Worktree: `/data/work/opensource/migo/.worktrees/three-platform-delivery`
- Branch: `delivery/three-platform-rc`
- Implementation base (HEAD before this documentation-only handoff commit):
  `28e59da08f4a7e2b8baa87165cbd0f4c0f074d2d`
  (`fix: enforce Android capability permissions`)
- Previous documentation-only handoff commit:
  `93c9a20f9cdab4181a63eeed48d6f097f167209d`
- Local `master`: `572d054087e324893a4608c431e4b4b0495189de`
- `origin/master`: `d41389dd4265fd68635f64951f653e349779cc0c`
- Required safety stash:
  `stash@{0}: On delivery/three-platform-rc: phase-a-before-master-d41389d`
- The index and worktree contain a large mixture of staged, unstaged, and new
  Phase A files. Do not broad-stage, reset, restore, or commit them blindly.
- No code checkpoint was committed at this stop because the P1 implementation
  had not completed fresh verification or independent review. No push, tag, or
  publication occurred.

### Interrupted P1: BLE callback admission and teardown

The open P1 is that an old Android GATT attempt must never deliver
characteristic read/change, MTU, or RSSI data after permission revocation,
terminal close, or a reconnect that replaced the attempt.

The completed part of the current WIP introduces a gate-owned counted callback
lease rather than a read-then-run scope query:

- `PermissionOperationGate.runIfGranted` admits a scope-local `activeRuns`
  lease under the permission-session monitor, releases that monitor for the
  callback body, and retires the lease in `finally`.
- Denial, targeted revoke, and terminal close publish false/closing and call
  `awaitIdle` before cancellation, native permission update, or resource
  teardown. The wait has no timeout and restores interrupt status only after
  the lease count drains.
- `NativeExports.runIfPermissionGranted` exposes the gate operation to the
  platform package.
- `BluetoothManager` wraps map identity, concrete GATT attempt, Android
  permission/session lifecycle, native characteristic/MTU dispatch, and
  MTU/RSSI cache mutation inside the `scope.bluetooth` lease.

The implementation touched exactly these P1 files in the last completed round:

1. `platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java`
2. `platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java`
3. `platforms/android/library/src/main/java/com/migo/runtime/internal/platform/BluetoothManager.java`
4. `platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java`
5. `platforms/android/library/src/test/java/com/migo/runtime/internal/platform/BluetoothManagerGattCleanupTest.java`

After deterministic remediation, focused Full and Slim execution completed:

```bash
./gradlew :library:testFullDebugUnitTest :library:testSlimDebugUnitTest \
  --tests '*PermissionOperationGateTest' \
  --tests '*BluetoothManagerGattCleanupTest'
```

Each profile passed 12 gate tests plus 11 GATT tests (23/23, no failures,
errors, or skips), and `git diff --check` was clean. The tests now include
callback-exception lease release, a positive false-state barrier before live
GATT teardown, all four sensitive callback/cache paths, and independent Android
connect-permission and terminal-session rejection. The independent spec
re-review returned **APPROVED** for that state.

The subsequent independent code-quality review returned **NOT APPROVED** with
one Important finding: after `publishGattConnection()` succeeded, the
connection-state callback released `GattAttempt` before `discoverServices()`
and the final state report. Concurrent revoke/terminal cleanup could therefore
close and remove the attempt, after which the callback could still use the
closed GATT and report `connected=true`.

The implementer began fixing that finding before the latest stop. The current
tree now contains additional, **unverified** edits:

- Connected state handling enters the `scope.bluetooth` admission, rechecks
  map identity, then calls `GattAttempt.dispatchIfActive`; discovery and final
  reporting execute while that attempt monitor is owned.
- A failed standing-scope/attempt admission reports `connected=false` without
  calling `discoverServices`.
- Three new focused fixtures were added for close winning before admission,
  callback-owned discovery preceding concurrent close, and standing-scope
  denial while the concrete GATT remains live.

The user interrupted the implementer before it reported RED, GREEN, compile,
self-review, or mutation evidence. Do not assume these partial edits work. No
spec re-review or quality re-review has inspected them. The next agent must
first inspect the exact diff and verify these invariants:

1. Connection discovery and final `connected=true` delivery are linearized
   with `beginClose`; neither can happen after teardown wins.
2. A callback that owns attempt admission completes reporting before concurrent
   teardown, without a gate/attempt/Rust lock cycle.
3. Standing-scope denial rejects connected discovery while preserving correct
   cleanup ownership and without double-close, lost-handle, or false-state
   reporting regressions.
4. Early `connectGatt` callbacks, sequential disconnect/failure retry paths,
   and existing sensitive callback/cache behavior remain correct.

The quality reviewer also recorded these nonblocking residual risks, which
must be resolved or performance-evidenced before external delivery:

- Every sensitive callback currently takes the global Java `sessions` monitor
  and allocates an `AndroidGattConnection` plus capturing lambdas. This is
  avoidable cross-session contention/allocation on a notification hot path and
  has no throughput/allocation regression test.
- Pending cancellation actions execute while holding the permission-session
  monitor. Current location cancellation has no known reverse lock edge, but
  the design can convoy unrelated scopes and is fragile for future cleanup.
- Several concurrency fixtures retain one-second scheduling deadlines or
  unbounded joins, which can flake or hang on loaded CI.

Do not commit the P1 until the interrupted connection-state changes have fresh
focused evidence, renewed spec approval, renewed quality approval, and fresh
full verification.

### Last known verification (stale for the current P1 WIP)

Before the final BLE callback edits, the following evidence was green:

- Android Full and Slim JVM unit suites: 72 tests each, no failures, errors, or
  skipped tests.
- Rust `migo-shared`, `migo-core`, `migo-platform`, and `migo-capi` library
  suites: 596 passed and one X11-native test ignored.
- Runtime V8 permission tests: four passed, 495 filtered out.
- Permission inventory: 30 gated APIs, eight cleanup APIs, 38 sensitive APIs;
  the coverage mutation self-test passed.
- Owned-host, Android host API baseline (356 entries), input transport, X11
  ownership, R8 static structure (5/5), and surface attachment (20/20)
  contracts passed.

These results do **not** verify the current tree. The later focused 23/23 result
and spec approval predate the interrupted connection-state edits, so they are
also stale for the files now on disk. Do not quote any of these as release
evidence until the P1 is independently re-approved and fresh full verification
passes.

The X11 native test's initial `xvfb-run` failure was environmental:
`/tmp/.X11-unix` is a read-only mount owned by `nobody:nogroup`, so Xvfb could
not bind the Unix socket. The same native test passed 1/1 against an isolated
TCP Xvfb on display `:223`; the X11 unit result was four passed and one ignored.
No product workaround was added for this host condition.

### Explicitly deferred findings

The user narrowed this stop to P1 only and explicitly deferred these P2 items:

1. If cleanup of a rejected late GATT connection fails, the concrete handle is
   not retained for a later retry.
2. `CameraSlot` publishes ownership before `mgr.create`; terminal cleanup can
   race initialization and must be made transactional.
3. Generated/final Full and Slim merged-manifest behavior for API 26, 28, and
   31 still needs shipping-AAR verification.

The real consumer R8 gate also remains deferred until real Full/Slim multi-ABI
AARs exist. The repository currently lacks the required Android V8/native
artifacts under `engine/jniLibs` / `platforms/android/dist` for both ABIs. The
correct Phase C design is a standalone `:r8-consumer` using the built AAR files
(never `project(':library')`), `minifyEnabled true`, no default/local keep rules
that mask the AAR consumer rules, and final DEX checks for exact JNI roots plus
a removable sentinel. It must also verify `classes.jar`, `proguard.txt`,
mapping/usage output, both ABI `libmigo`/`libc++` entries, and ELF identity. Do
not use fake `.so` files, classes-only AARs, `--skip-rust`, or host archives.

## Resume Audit

- [ ] **Step 1: Establish the exact checkpoint**

Run these commands separately from the delivery worktree and save their output
in the task notes:

```bash
git status --short --branch
git log --oneline --decorate -12
git diff --stat
git diff --cached --stat
git stash list
git rev-parse master
git rev-parse origin/master
git branch -vv
git ls-remote --heads origin delivery/three-platform-rc
```

Expected: the delivery branch contains the local checkpoint commits and the
named Phase A safety stash still exists. `git branch -vv` records local upstream
configuration; only the `git ls-remote` result can establish whether the named
delivery branch exists on the remote. It must print no matching remote ref. Do
not infer remote publication state from local history alone. Do not assume an
empty index or worktree; classify every remaining path before editing or
staging it.

- [ ] **Step 2: Verify the checkpoint rather than trusting its message**

Run the exact focused tests named by the checkpoint commit and inspect their
test counts. At minimum, re-run permission coverage and its mutation self-test,
Android Full/Slim permission tests, Rust permission behavior and concurrency
tests, `cargo fmt --all --check`, and `git diff --check` if the checkpoint
contains A7.

- [ ] **Step 3: Reconcile the ledger**

For each A1-A13 checkbox, compare the implementation, tests, review findings,
and fresh evidence. Change `[ ]` to `[x]` only when all of those agree. A commit
or a source-text contract by itself is not completion evidence.

- [ ] **Step 4: Establish a task-owned commit boundary**

Before implementing a task, require an empty index or an index containing only
that task's reviewed files in an isolated worktree. Before committing, compare
`git diff --cached --name-only` against the task file list and run
`git diff --cached --check`. Never run a broad `git add` while unrelated paths
are staged, and never reset, restore, or discard user work to obtain a clean
index.

## Remaining Execution Order

- [ ] **Task 1: Finish Phase A correctness in ledger order**

Close A1-A13 before starting release packaging. Preserve the ownership designs
already written for Host threads, platform identity, X11, and bounded input.
Split additional detailed TDD plans before implementing A10-A12; each plan must
name exact files, RED commands, GREEN commands, native/JVM behavioral tests,
and a local commit boundary.

Required Phase A outcomes include:

- Engine destruction joins every owned thread; Android shutdown has no timeout
  escape; X11/Wayland/Win32 resources are caller-releasable only after the
  documented barrier.
- Runtime restart uses Host-lifetime callback IDs, exact result correlation,
  generation-bearing callbacks before every JavaScript dispatch,
  expected-generation resource admission, retired-Worker join, synchronous
  platform/audio/render fences without timeout escape, and transactional
  publication as specified by the 12-task restart plan.
- Canvas recovery handles the complete save/clip/path state, pattern resources,
  explicit main-canvas dimensions, and every snapshot failure exit.
- Standing permissions can be seeded before startup, trusted descriptions come
  from validated declarations, deferred framework entry is linearized with
  revocation, and Full/Slim manifests match API 26/28/31 behavior.
- Retained bridge intrinsics, module loading, ads, reliable results, pixel ratio,
  Windows identity, public API baselines, and post-master integration conflicts
  all have behavioral or native contract evidence.

- [ ] **Task 2: Implement Phase B hermetic builds and packages**

Execute B1-B9 from the delivery ledger. Materialize V8, Skia, and ANGLE by
verified component identity before Cargo or linking. The currently missing
Android archive at
`engine/third_party/rusty_v8/aarch64/librusty_v8.a` must be produced or fetched
through the repository's verified Android V8 component flow; never replace it
with the Linux host archive or an empty archive.

The phase ends only after deterministic Android, Linux, and Windows archives,
package manifests, checksums, notices, BSL text, SBOMs, and provenance inputs
verify from clean target directories and same-source rebuilds compare equal.

- [ ] **Task 3: Run Phase C native qualification**

Qualify Android API 26+ `arm64-v8a` and `x86_64`, Linux glibc x86_64 on X11 and
Wayland, and Windows 10/11 x86_64 with Win32/ANGLE. Build external consumers
against installed artifacts, reach a verified first frame, exercise resize,
input, background/surface recreation, detach, shutdown, and process exit, and
archive exact toolchain and thread-clean evidence.

- [ ] **Task 4: Repair and qualify `migo-examples`**

Execute D1-D8 against candidate artifacts rather than repository-relative
build outputs. Obtain write approval for the adjacent repository when needed.
Keep resolver trust, authentication relay, Windows process invocation, content
identity, input semantics, and lifecycle cleanup fail-closed. Add all three
platform CI consumers and prove first frame from a clean examples checkout.

- [ ] **Task 5: Produce performance evidence and public release material**

Execute E1-E7. Benchmark representative content on a physical Android device
against System WebView under controlled conditions, collect the required
startup/frame-time/memory/CPU/thermal/energy/size data, and reject missing or
zero-sample reports. Only claims supported by committed evidence belong in the
README. Document the source-available position, WebView-alternative scope,
mini-game and HTML5/Canvas/WebGL integration, Android/Linux/Windows matrix, and
tested integration path.

- [ ] **Task 6: Complete E8 final independent delivery audit**

From the assembled candidate directory, re-verify hashes, manifests, SBOM,
licenses, exports, ABI/API floors, deterministic archives, installed consumers,
native smoke evidence, examples, documentation consistency, and every required
CI dependency. Stop at a locally verified candidate; do not tag or publish it.

- [ ] **Task 7: Complete E9 local candidate commit**

After every E8 finding is closed and all evidence has been rerun from the
assembled candidate directory, commit the verified `0.10.0-rc.1` state locally.
Record the commit and verification evidence in the delivery ledger. Do not push,
tag, publish, or delete the safety stash.

## Prompt For The Next Agent

Use the following prompt verbatim in a new capable coding-agent session:

```text
Resume the active Migo three-platform release goal from the uncommitted WIP.
Work only in
/data/work/opensource/migo/.worktrees/three-platform-delivery on
delivery/three-platform-rc, and also review/fix
/data/work/opensource/migo-examples when Phase D is reached. Do not push, tag,
publish, delete stashes, fake native dependencies, or use workarounds.

First read, in order:
1. docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md
2. docs/superpowers/plans/2026-07-29-three-platform-delivery.md
3. docs/superpowers/plans/2026-08-02-runtime-restart-generation-boundary.md
4. the A1-A4 detailed plans referenced by the delivery ledger.
5. docs/superpowers/plans/2026-08-02-three-platform-delivery-handoff.md

Then inspect git status and every current diff before editing. Preserve the
phase-a-before-master-d41389d stash. The current tree is NOT a verified
checkpoint. The gate-owned counted callback lease and its deterministic
exception/false-state/OS/terminal tests previously passed focused Full and Slim
23/23 and received spec APPROVED. The following quality review found one
Important connection-state race, and the implementer was interrupted while
fixing it. Current source and tests contain partial edits with no reported RED,
GREEN, compile, self-review, spec re-review, or quality re-review.

Audit PermissionOperationGate, NativeExports, BluetoothManager,
PermissionOperationGateTest, and BluetoothManagerGattCleanupTest first. In
particular inspect the new connected-state path that wraps map identity,
`GattAttempt.dispatchIfActive`, discovery, and final reporting in the standing
scope admission/attempt monitor, plus the new session-18/19/20 concurrency
fixtures. Do not replace the counted lease with a read-only scope query.

Re-establish deterministic TDD evidence for the interrupted connection-state
change. Use a targeted mutation or equivalent RED to prove the tests fail if
discovery/reporting run after `beginClose`, if the standing-scope gate is
bypassed, or if teardown passes a callback that owns attempt admission. No
polling, spin, sleep, production timeout, false success, double-close, or lost
cleanup handle. Preserve early `connectGatt` callback behavior and the existing
sensitive callback/cache contract.

Run focused Full/Slim tests and diff-check. Then require renewed spec approval,
followed by a separate code-quality approval, before full Android Full/Slim and
relevant Rust/contract regression verification. Do not trust the prior 72/72,
596-pass, 7/7, 20/20, or 23/23 as evidence for the partial tree now on disk.
Track the quality review's residual hot-path global lock/allocation,
cancellation-under-monitor, and concurrency-test timeout/join risks; close or
performance-evidence them before external delivery.

After P1 is independently approved and locally committed at an isolated file
boundary, address the explicitly deferred P2 findings recorded in this handoff:
retained retry for rejected late GATT close failure, transactional camera
create-vs-terminal-cleanup, and generated/final Full/Slim merged-manifest API
26/28/31 verification. Use strict TDD, systematic debugging, a spec review
followed by a code-quality review for every task, and fresh verification before
every local commit.

Finish A1-A13 before Phase B, including the 12-task runtime restart plan and
detailed TDD plans for Canvas recovery and remaining permission/platform gaps.
Then execute Phases B-E through a locally verified Android API 26+
arm64-v8a/x86_64, Linux glibc x86_64 X11/Wayland, and Windows 10/11 x86_64
Win32/ANGLE 0.10.0-rc.1 candidate. Required gates fail closed. If a real tool or
SDK installation is the only blocker, stop and give the user the exact install
command. Otherwise continue autonomously using the best technically correct
design. Build the actual consumer R8 gate only after real Full/Slim multi-ABI
AARs exist; never use fake `.so`, classes-only AARs, `--skip-rust`, or host
archives for Android. Commit verified stages locally but never push.
```
