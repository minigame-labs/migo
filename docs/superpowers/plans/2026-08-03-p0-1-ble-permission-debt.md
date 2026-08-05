## P0.1 BLE Permission-Path Locking Debt Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the two locking defects on the Android BLE permission admission path — cancellation actions running under the permission-session monitor, and a cross-session lock on the per-event callback path — then obtain the outstanding independent reviews for the connection-state fixes already committed.

**Architecture:** `PermissionOperationGate` keeps its two-level locking (a per-session transition lock that serialises grant, revoke, and close, and a per-session monitor that guards scope state and the counted callback lease). The change narrows the monitor: it still covers state mutation and the lease drain, but foreign cancellation actions execute after it is released while the transition lock is still held, which is sufficient because a denied or closing scope can admit no new lease or registration. Session lookup moves to a concurrent map so a BLE notification no longer serialises against every other session.

**Tech Stack:** Java 8 source level, JUnit 4, Gradle with Full and Slim product profiles.

---

## Context

Read `docs/superpowers/plans/2026-08-03-p1-ble-admission-status.md` first. It
records what the committed connection-state work proved and what remains open.

Files in scope:

- `platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java`
- `platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java`

Do not change `BluetoothManager.java` in this task. Its allocation behaviour is
task 5.1.

Verification commands used throughout:

```bash
cd platforms/android
./gradlew --offline -q \
  :library:testFullDebugUnitTest --tests '*PermissionOperationGateTest' \
  :library:testSlimDebugUnitTest --tests '*PermissionOperationGateTest'
```

`--tests` binds only to the task it follows, so both tasks need their own
filters. Read exact counts from
`library/build/test-results/test<Profile>DebugUnitTest/*.xml` rather than
trusting the build status.

## Task 1: Prove cancellation currently runs under the session monitor

**Files:**
- Test: `platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java`

- [ ] **Step 1: Write the failing test**

Append this test. It registers a cancellation that blocks, then proves an
unrelated scope operation on another thread can still make progress while that
cancellation runs. Under the current implementation the cancellation holds the
session monitor, so the unrelated registration blocks and the latch times out.

```java
    @Test
    public void closeCancellationDoesNotRetainTheSessionMonitor() throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2004));
        assertNull(gate.update(2004, "scope.bluetooth", true, () -> true).failure());
        assertNull(gate.update(2004, "scope.camera", true, () -> true).failure());
        CountDownLatch cancellationEntered = new CountDownLatch(1);
        CountDownLatch releaseCancellation = new CountDownLatch(1);
        assertNotNull(gate.register(2004, "scope.bluetooth", () -> {
            cancellationEntered.countDown();
            await(releaseCancellation);
        }));

        Thread close = new Thread(() -> gate.close(2004));
        close.start();
        assertTrue(cancellationEntered.await(1, TimeUnit.SECONDS));

        CountDownLatch observed = new CountDownLatch(1);
        Thread observer = new Thread(() -> {
            gate.retainedSessionCountForTests();
            observed.countDown();
        });
        observer.setDaemon(true);
        observer.start();

        try {
            assertTrue(
                    "a blocked cancellation retained the permission-session monitor",
                    observed.await(1, TimeUnit.SECONDS));
        } finally {
            releaseCancellation.countDown();
        }
        joinBounded(close);
        joinBounded(observer);
    }
```

- [ ] **Step 2: Run it and confirm it fails for the intended reason**

```bash
cd platforms/android && ./gradlew --offline -q \
  :library:testFullDebugUnitTest --tests '*PermissionOperationGateTest'
```

Expected: this one test fails. Confirm the failure message is
`a blocked cancellation retained the permission-session monitor`, not a
compilation error and not a different test.

If the test passes as written, the observation point is too weak. Replace
`retainedSessionCountForTests()` with an operation that provably needs the
session monitor — `gate.register(2004, "scope.camera")` — and assert the returned
`Pending` is null because the session is closing, which still requires entering
the monitor.

## Task 2: Narrow the monitor around cancellation

**Files:**
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java`

- [ ] **Step 1: Replace the static `cancelAll` with a snapshot-then-run pair**

Delete `cancelAll` and add these two members. `snapshotPending` runs under the
session monitor and only copies; `runCancellations` runs with the monitor
released and reacquires it briefly to retire each success, so a failure keeps its
`Pending` for a later retry exactly as before.

```java
    private static ArrayList<Pending> snapshotPending(Entry entry) {
        return new ArrayList<>(entry.pending);
    }

    private static void runCancellations(Session session, Entry entry) {
        ArrayList<Pending> snapshot;
        synchronized (session) {
            snapshot = snapshotPending(entry);
        }
        ArrayList<ResourceCleanup.Action> cancellations = new ArrayList<>();
        for (Pending pending : snapshot) {
            cancellations.add(() -> {
                pending.cancellation.run();
                synchronized (session) {
                    pending.active = false;
                    entry.pending.remove(pending);
                }
            });
        }
        ResourceCleanup.runAll(cancellations.toArray(new ResourceCleanup.Action[0]));
    }
```

- [ ] **Step 2: Call it outside the monitor in `update`**

Replace the denial branch body so the cancellation no longer runs inside
`synchronized (session)`:

```java
                } else {
                    ResourceCleanup.runAll(
                            () -> runCancellations(session, entry),
                            () -> requireNativeSuccess(updateNative));
                }
```

- [ ] **Step 3: Call it outside the monitor in `revoke`**

Rewrite `revoke` so state mutation and the drain stay under the monitor while the
cancellation runs after it is released. The transition lock is still held, so no
new lease or registration can appear.

```java
    public Result revoke(int sessionId, String scope) {
        Session session = session(sessionId);
        if (session == null) return new Result(null);
        synchronized (session.transition) {
            Entry entry;
            synchronized (session) {
                entry = session.scopes.get(scope);
                if (entry == null) return new Result(null);
                entry.granted = false;
                awaitIdle(session, entry);
            }
            try {
                runCancellations(session, entry);
                return new Result(null);
            } catch (RuntimeException error) {
                return new Result(error);
            }
        }
    }
```

- [ ] **Step 4: Call it outside the monitor in `close`**

Rewrite the body of `close` between the transition lock and the session removal:

```java
            Result result;
            ArrayList<Entry> entries;
            synchronized (session) {
                session.lifecycle = Lifecycle.CLOSING;
                for (Entry entry : session.scopes.values()) {
                    entry.granted = false;
                }
                entries = new ArrayList<>(session.scopes.values());
                for (Entry entry : entries) {
                    awaitIdle(session, entry);
                }
            }
            ArrayList<ResourceCleanup.Action> cancellations = new ArrayList<>();
            for (Entry entry : entries) {
                cancellations.add(() -> runCancellations(session, entry));
            }
            try {
                ResourceCleanup.runAll(
                        cancellations.toArray(new ResourceCleanup.Action[0]));
                result = new Result(null);
            } catch (RuntimeException error) {
                result = new Result(error);
            }
```

- [ ] **Step 5: Run the focused tests on both profiles**

```bash
cd platforms/android && ./gradlew --offline -q \
  :library:testFullDebugUnitTest --tests '*PermissionOperationGateTest' \
  :library:testSlimDebugUnitTest --tests '*PermissionOperationGateTest'
```

Expected: all tests pass, including the new one and the pre-existing
`failedCloseCancellationIsRetainedAndRetriedAcrossAllScopes`,
`updateWaitingBehindCloseCannotReopenTheSession`,
`closeDrainsAdmittedScopeRunBeforeCancellationAndRejectsLaterRun`, and
`successfulCloseReclaimsSessionsWithoutAllowingIdReuse`. Those four pin the retry
retention, the tombstone, the drain ordering, and session reclamation, which are
exactly the properties this restructure could break.

- [ ] **Step 6: Commit**

```bash
git add platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java \
        platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java
git commit -m "fix(android): run permission cancellations outside the session monitor"
```

## Task 3: Prove the per-event path takes a cross-session lock

**Files:**
- Test: `platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java`

- [ ] **Step 1: Write the failing test**

Two different sessions must be able to admit callbacks concurrently. Under the
current implementation `session(int)` synchronises on the shared map, so a
callback that holds its own session's lease still blocks another session's
lookup only if the map monitor is held — which it is not, because the lookup
releases it. The observable defect is therefore that a *long-running* map
operation serialises unrelated sessions. Make that concrete by opening one
session while another session's callback is in flight, since `open` holds the map
monitor for its whole body.

```java
    @Test
    public void oneSessionCallbackAdmissionDoesNotSerialiseAnotherSession()
            throws Exception {
        PermissionOperationGate gate = new PermissionOperationGate();
        assertTrue(gate.open(2005));
        assertNull(gate.update(2005, "scope.bluetooth", true, () -> true).failure());
        assertTrue(gate.open(2006));
        assertNull(gate.update(2006, "scope.bluetooth", true, () -> true).failure());

        CountDownLatch firstEntered = new CountDownLatch(1);
        CountDownLatch releaseFirst = new CountDownLatch(1);
        Thread first = new Thread(() -> gate.runIfGranted(2005, "scope.bluetooth", () -> {
            firstEntered.countDown();
            await(releaseFirst);
            return true;
        }));
        first.start();
        assertTrue(firstEntered.await(1, TimeUnit.SECONDS));

        CountDownLatch secondAdmitted = new CountDownLatch(1);
        Thread second = new Thread(() -> {
            if (gate.runIfGranted(2006, "scope.bluetooth", () -> true)) {
                secondAdmitted.countDown();
            }
        });
        second.setDaemon(true);
        second.start();

        try {
            assertTrue(
                    "a callback in one session blocked admission in another session",
                    secondAdmitted.await(1, TimeUnit.SECONDS));
        } finally {
            releaseFirst.countDown();
        }
        joinBounded(first);
        joinBounded(second);
    }
```

- [ ] **Step 2: Run it**

```bash
cd platforms/android && ./gradlew --offline -q \
  :library:testFullDebugUnitTest --tests '*PermissionOperationGateTest'
```

Record the result. If it passes, the cross-session lock is not observable through
this path alone, and the remaining justification for Task 4 is that the shared
monitor is on the per-event path at all, which specification Section 7.3
prohibits regardless of whether a fixture can catch it. In that case keep the
test as a permanent guard, note in the commit that it was green before the
change, and proceed to Task 4.

## Task 4: Make session lookup lock-free

**Files:**
- Modify: `platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java`

- [ ] **Step 1: Replace the guarded map with a concurrent map plus an open guard**

```java
    private final java.util.concurrent.ConcurrentHashMap<Integer, Session> sessions =
            new java.util.concurrent.ConcurrentHashMap<>();
    private final Object openGuard = new Object();
    private int highestOpenedSessionId = -1;
```

- [ ] **Step 2: Guard only the monotonic open invariant**

```java
    public boolean open(int sessionId) {
        synchronized (openGuard) {
            if (sessionId <= highestOpenedSessionId || sessions.containsKey(sessionId)) {
                return false;
            }
            highestOpenedSessionId = sessionId;
            sessions.put(sessionId, new Session());
            return true;
        }
    }
```

- [ ] **Step 3: Make lookup, removal, and the test accessor lock-free**

```java
    private Session session(int sessionId) {
        return sessions.get(sessionId);
    }
```

In `close`, replace the guarded removal with the atomic two-argument form:

```java
            if (result.failure() == null) {
                sessions.remove(sessionId, session);
            }
```

And the test accessor:

```java
    int retainedSessionCountForTests() {
        return sessions.size();
    }
```

- [ ] **Step 4: Run the focused tests on both profiles**

```bash
cd platforms/android && ./gradlew --offline -q \
  :library:testFullDebugUnitTest --tests '*PermissionOperationGateTest' \
  :library:testSlimDebugUnitTest --tests '*PermissionOperationGateTest'
```

Expected: all pass. `duplicateOpenCannotRepublishAnExistingSession`,
`closeLeavesTombstoneAndCannotRetainGrantOrBeReopened`, and
`successfulCloseReclaimsSessionsWithoutAllowingIdReuse` pin the invariants this
step could break.

- [ ] **Step 5: Commit**

```bash
git add platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java \
        platforms/android/library/src/test/java/com/migo/runtime/internal/PermissionOperationGateTest.java
git commit -m "perf(android): make permission session lookup lock-free on the callback path"
```

## Task 5: Mutation evidence

- [ ] **Step 1: Prove Task 2 is load-bearing**

Temporarily wrap `runCancellations`'s body in `synchronized (session) { ... }` so
the cancellation again runs under the monitor. Run the focused tests and confirm
`closeCancellationDoesNotRetainTheSessionMonitor` fails. Revert, and confirm the
production file is byte-identical to its committed state:

```bash
git diff --exit-code -- platforms/android/library/src/main/java/com/migo/runtime/internal/PermissionOperationGate.java
```

- [ ] **Step 2: Prove the drain is still load-bearing**

Change `awaitIdle`'s loop condition to `entry.activeRuns < 0` so it never waits.
Run the focused tests and confirm the three drain fixtures fail:
`denialDrainsAdmittedScopeRunBeforeNativeUpdateAndRejectsLaterRun`,
`closeDrainsAdmittedScopeRunBeforeCancellationAndRejectsLaterRun`, and
`standingScopeDenialDrainsAdmittedCallbackBeforeClosingGattAndRejectsLateData`
in the Bluetooth suite. Revert and re-verify byte equality.

## Task 6: Full verification

- [ ] **Step 1: Run the complete Android suites on both profiles**

```bash
cd platforms/android && ./gradlew --offline -q \
  :library:testFullDebugUnitTest :library:testSlimDebugUnitTest
```

Expected: 86 tests per profile plus the two tests added here, no failures,
errors, or skips. Read the counts from the XML reports.

- [ ] **Step 2: Run the permission coverage contract**

```bash
bash scripts/test-permission-coverage-contract.sh
```

Expected: `PASS: permission coverage contract (30 gated op(s), 8 cleanup op(s),
38 permission-sensitive op(s))`.

- [ ] **Step 3: Run the Rust library suites**

```bash
cd engine
for p in migo-shared migo-core migo-platform migo-capi; do
  cargo test -p $p --lib --locked --offline; done
```

Expected: 596 passed, one ignored X11 native test.

- [ ] **Step 4: Check whitespace**

```bash
git diff --check
```

## Task 7: Independent reviews

- [ ] **Step 1: Spec-compliance review**

Dispatch an independent reviewer with no context from the implementing session.
It must judge this change plus the two connection-state fixes already committed
against `docs/superpowers/specs/2026-08-03-four-platform-delivery-design.md`
Sections 6.1 and 7.3, and against Section 3.1 of the inherited three-platform
design. Require a verdict and findings graded Critical, Important, and Minor with
file and line citations.

- [ ] **Step 2: Code-quality review**

Only after the spec review is approved, dispatch a separate independent
code-quality review covering lock discipline and deadlock freedom across the
transition lock, the session monitor, the `GattAttempt` monitor, the Android
framework calls made while holding a monitor, and the Rust side reached through
`NativeMethods`; the counted lease's exception, interrupt, and missed-wakeup
behaviour; whether `awaitIdle` can be starved; and fixture determinism.

- [ ] **Step 3: Close every Critical and Important finding and repeat both
  reviews as required.**

- [ ] **Step 4: Mark the ledger**

Set task 0.1 to `- [x]` in
`docs/superpowers/plans/2026-08-03-four-platform-delivery.md` only when
implementation, tests, fresh verification, and both approvals all agree. Record
the exact test counts.
