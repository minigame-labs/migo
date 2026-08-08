> Part of the [Four-Platform Delivery Ledger](../2026-08-03-four-platform-delivery.md).

## Tooling And Verification

- [x] T.1 Make the Android SDK's Java half a target the verifier knows about.
  `scripts/verify-change.sh` contained no reference to gradle, java or
  `platforms/` at all: asked what
  `platforms/android/.../BluetoothManager.java` requires, the selector returned an
  **empty plan**. A change to the shipped AAR's own sources therefore ran eleven
  Rust suites, cross-compiled Rust for Android, and printed "verified for every
  target this change touches" without compiling a line of Java — the same defect
  the script's own header says it exists to prevent, one layer out. Task 0.24's
  Java-only fix was verified by hand for exactly this reason.

  `android-java` is a lane in `verification_targets.py` rather than a tier on
  `android`, because tiers replace each other and a change touching both halves
  needs both builds. Any path under `platforms/android/` asks for it, deliberately
  without enumerating which files matter — Gradle's inputs include manifests,
  resources and the build scripts, and a list is a thing to forget an entry from.
  Both product variants run, because the Java sources are variant-independent while
  `BuildConfig` capability gating is not. The lane is **probed**: a machine without
  `gradlew` reports NOT PROVEN like every other absent target, rather than FAIL,
  which would say "your change broke this" about missing evidence.
  `test-local-verification-contract.sh` grew four assertions (59 checks from 54),
  including that a Gradle build script is an input.

- [x] T.2 Adopt `cargo-mutants`, and fix what it found immediately.
  This ledger's mutation evidence has been produced by hand-written apply/restore
  scripts. `cargo-mutants 27.1.0` runs through `dev-test-host.sh` (which passes any
  cargo subcommand through, so the host toolchain is inherited) and reports
  survivors, which is the artifact these entries are actually made of.

  **Its first run found a real hole in work committed one hour earlier.** Scoped to
  `crates/shared/src/payload_pool.rs`: **13 of 25 mutants survived**, including
  `try_acquire -> None`, `Drop for Recycled -> ()`, and the capacity check's `==`
  flipped to `!=`. The cause was not the mutants being exotic — it was that every
  test for the new `RecyclePool` lived in `migo-core`, one crate away from the
  mechanism, while the older `PayloadPool` beside it has its own. Five tests later
  (buffer identity across a return, high-water-mark growth, refusal at capacity,
  zero-capacity construction, and a steady-state allocation burst over the pool
  itself) it reports **2 survivors, both `Debug::fmt`** — nothing asserts on debug
  output, so those are noise. migo-shared 419 tests from 414.

  **One assertion in those tests was wrong and the code was right**, which is worth
  recording: it required a kept buffer's capacity to equal five after
  `extend_from_slice(b"hello")`, and `Vec` reserves eight. Buffer *identity* is the
  property; a capacity is only evidence about `Vec`'s growth strategy.

  Usage and scoping rules are in
  `docs/superpowers/plans/2026-08-08-four-platform-delivery-handoff.md`.

- [ ] T.3 Make the host suites selective. **Implemented, measured and pinned in both
  directions; neither independent review has run, so the item stays open.**

  Sixteen host suites ran on every invocation — the count grew when task 0.15 added
  the two Slim ones — so a Java-only change paid for the whole Rust tree. The
  selector now answers with the changed packages plus their **reverse-dependency
  closure**, taken from `cargo metadata --no-deps`: a change to a leaf crate still
  runs the suites of everything that depends on it, because that is where its
  behaviour is observed. Dev-dependencies are edges too — a crate whose *tests* use
  another crate is a crate whose suite a change to that other crate can break.

  **The dangerous direction is the quiet one**, so every branch fails towards running
  more. `HOSTSUITES ALL` is the answer for a tree `cargo metadata` cannot describe,
  for a file under `engine/` belonging to no workspace member (a lock file, a
  workspace manifest), and for any path outside `engine/` that is not provably
  irrelevant — `scripts/` decides how the suites run at all. A missing `HOSTSUITES`
  line lands in the same branch. `build --workspace --all-targets` and
  `fmt --all --check` stay whenever any package is implicated, because several
  members have no suite of their own and that build is the only thing compiling them.
  `NONE` keeps `fmt` alone: it costs a second, and a stray unformatted file is worth
  catching wherever it came from.

  Measured closures: a change to `crates/shared` selects 12 packages (it is the root
  of the graph, so there is nothing to save there and the selector correctly says so),
  `crates/audio` selects 7, `crates/capi` selects 2, a Java-only change selects none.
  End to end, a Java-only change now takes **2m0s** against roughly thirteen minutes
  before — the remaining time is the contract lane and both Gradle flavours, which is
  what such a change actually needs.

  Pinned in both directions, and each mutant dies at the check named for it: deleting
  the closure fails *"a leaf change reaches the crates that depend on it"* and *"the
  dependent's suite is actually run, not merely planned"*; treating an unknown path as
  needing nothing fails *"a change outside engine/ that could affect anything runs
  every suite"*. The fixture needed one real dependency edge for this — with
  standalone stub crates, under-running and correct behaviour produce identical
  output, which is the shape of a gate that cannot fail.

  **The contract lane stays unconditional** (task T.6). Each of its gates is seconds,
  and keying them to changed files would mean a file list per gate — a list to forget
  an entry from, which is how a gate stops covering what it names.

- [ ] T.4 Add `pitest` for the Java half, for the reason T.2 gives for the Rust half.
  **Wired and run; the survivors it found are listed below and are not yet fixed, so
  the item stays open on its own findings rather than on its tooling.**

  **The published Gradle plugin cannot be used here, and the reason is structural.**
  `info.solidsoft.pitest` registers its `pitest {}` extension inside a
  `plugins.withType(JavaPlugin)` callback, and `com.android.library` never applies
  `JavaPlugin` — the build fails with "Could not find method pitest()". PIT's own
  command-line entry point (`org.pitest.mutationtest.commandline.MutationCoverageReport`)
  is what the plugin drives anyway, so `:library:pitestFullDebug` is a `JavaExec` onto
  that class with the JARs in their own `pitestRuntime` configuration, which keeps them
  out of the AAR and out of every `--offline` build that does not invoke the task.

  **Three findings worth keeping, because each one made the run silently wrong first.**
  `testRuntimeClasspath` does not exist in an Android module, and resolving the
  variant-scoped `fullDebugUnitTestRuntimeClasspath` from inside the same project trips
  an attribute-ambiguity error (Android tries to consume `:library` as its own
  dependency) — borrowing `testFullDebugUnitTest.classpath` is what works. `--classPath`
  takes **one entry per argument**: a single colon-joined string is accepted and treated
  as one opaque element, after which pitest finds zero classes and reports a clean run
  over nothing. And the task is declared `notCompatibleWithConfigurationCache`, because
  reading another task's resolved classpath at execution time is precisely what the
  cache forbids; resolving it at configuration time instead would make every build,
  including offline ones that never run pitest, resolve this variant's test classpath.

  **Read test strength, not the mutation score.** Most of `com.migo.runtime.internal`
  touches `android.*` and cannot be loaded by a host JVM at all — this module has no
  Robolectric by design, so 4,004 of 4,433 mutations report no coverage and drag the
  score down by a constant that says nothing about the tests. Excluding the test classes
  themselves mattered too: with them included the survivor count was 845, almost all of
  them mutated assertions, which buries the findings.

  Measured: `./gradlew :library:pitestFullDebug` — 4,433 mutations, 429 covered, 368
  killed, 4 timed out, **57 survivors, 85.8% test strength**. Survivors by class:
  `TouchEventHandler` 19, `BluetoothManager` 7, `PermissionOperationGate` 7,
  `NativeMethods` 6, `NativeExports$SessionPermissionSink` 3,
  `BluetoothManager$GattAttempt` 3, `VsyncSchedulerState` 2, `PermissionRevocation` 2,
  `TouchInputNormalizer` 2, one each in `LifecycleRequestState`,
  `LocationProvider$RetainedRequest`, `BluetoothManager$CharacteristicDispatch`.

  **The survivors were then worked through, and the useful artifact is the triage rather
  than the count.** Of the original 57, only about a third could ever be killed:

  - **~12 are harness-bound.** The `BluetoothManager` family and `LocationProvider`
    reach `Activity`, `BluetoothAdapter` and `Build.VERSION.SDK_INT`, and the stub
    `android.jar` reports `SDK_INT == 0`. Below API 31 the connect policy ignores the
    grant entirely, so *no* mutation of `hasConnectPermission`'s grant computation is
    observable here. The policy itself is a pure function and is fully tested; only the
    wiring is invisible. Adding a production seam to kill such a mutant is the
    anti-pattern this ledger already rejected once, so these stay recorded rather than
    chased.
  - **7 are equivalent mutants -- unkillable by construction.**
    `BooleanFalseReturnValsMutator` applied to a line that already reads `return false`
    produces identical bytecode, and pitest does not filter these. Five sit on
    `PermissionOperationGate`'s guard clauses (`return null`, `return false`,
    `return true`) and two on `TouchInputNormalizer`'s clamp, where `>` against `>=` at
    exactly `0.0f` and `<` against `<=` at exactly `1.0f` both yield the same value. A
    test written to kill one of these would be a test that cannot fail.
  - **The rest were real, and are now killed.** What they had in common is worth more
    than the list: every one was a *verdict a caller acts on* that the suite never
    asserted, because it asserted side effects instead. The gates drove their state
    machines thoroughly and then looked at what changed rather than at what was
    returned — and the caller does not see the side effect, it sees the boolean, which
    `NativeExports` hands straight to a JNI return.

  Killed in this round: `PermissionOperationGate.enter` and `runIfGranted` (admitted and
  refused now report opposite verdicts, and `runIfGranted` forwards the callback's own
  verdict); `PermissionRevocation.update` (both polarities, and a refusal reaches nothing);
  `NativeMethods.updatePermission` (one guard clause at a time, plus session id 0, which
  is the only case telling `>= 0` from `> 0`); `TouchEventHandler` in full — the packed
  flags against the Web Touch Events contract, the y coordinate that had no assertion at
  all, the pack loop's bound, and `updateDensity`'s fail-closed validation, which is the
  Java-side twin of the host pixel-ratio property in item 0.12;
  `NativeExports$SessionPermissionSink` (a successful grant settles quietly, and the
  suppressed "failed to schedule terminal close" appears exactly when the close could not
  be posted); `TerminalCloseQueue` to 100%; `LifecycleStateSynchronizer` (its interleaving
  test had a second writer, so deleting the application outright left the assertion true);
  `VsyncSchedulerState`, `TerminalCleanupState.Result` and `LifecycleRequestState`
  accessors, each of which had only ever been asserted in one state.

  Measured: **57 survivors to 20, test strength 85.8% to 94.5%**, Java 143 tests per
  flavour from 126. What remains is 12 harness-bound, 7 equivalent, and one marginal
  (`LifecycleRequestState`'s no-op `Action.NONE` return) -- so everything killable in this
  harness is killed. Two fixture faults were found on the way and are the transferable
  part: an assertion that two writers can both satisfy attributes nothing, and a test that
  only ever drives a failing dependency cannot tell "reports failure correctly" from
  "always reports failure".

  **The run is also now cheap enough to use per edit.** `-PpitestClasses` narrows it to
  one class — twelve seconds against about eight minutes for the package — so the loop is
  a focused run per change and one full run per batch.


- [x] T.5 Split this file. At ~5,500 lines it burned context on every read and was
  a merge-conflict magnet — the single biggest obstacle to more than one agent
  working at once. **Done.** The original path is now an index holding the Status
  Convention (other documents point at it there) plus a table of contents; the body
  lives in `2026-08-03-four-platform-delivery/part-{tooling,phase-0,phase-1,phases-2-5-blocked}.md`.
  Item identifiers are unchanged. No content loss was verified by comparing sorted
  line multisets of the committed original against the index plus every part: zero
  lines present at `HEAD` are absent from the split.

- [ ] T.6 Make the contract gates a lane the verifier knows about. **Implementation
  and evidence are done and recorded here; neither independent review has run, so
  the item stays open.**

  **This is T.1 one layer further out, and A12's own mutation evidence found it.**
  `verify-change.sh` ran host suites and target builds and had no concept of the
  two dozen source-structure contract gates that live in
  `.github/workflows/pr-ci.yml`. Those gates cover what a test cannot reach: what a
  crate may depend on, which resolver an entry point calls, whether an event's
  payload keys match its reader. Reverting one ad entry point to its bare handler
  lookup — the defect task 0.12 fixed — leaves **every unit test in both languages
  green** and is caught only by `scripts/test-ad-reward-integrity-contract.sh`. So
  the local gate printed "verified for every target this change touches" for a
  change CI rejects: the same sentence, about the same kind of blind spot, that T.1
  removed for Java.

  **The gate list is derived from the workflow, not restated.**
  `scripts/lib/ci_contract_gates.py` parses the `quality-gate` job and emits one
  `<disposition> <command>` line per invocation, keeping the environment
  assignments CI puts in front of them — a weaker local invocation of the same
  script is a quieter gate wearing the same name. A hand-maintained second copy
  would drift, and in the direction that matters: a gate added to CI and not here
  is a gate the local run silently does not have. The parser is stdlib-only and has
  an anti-vacuity floor — fewer than fifteen derived gates is a parse that has
  stopped matching, and it fails rather than reporting a lane with nothing in it.

  **Dispositions, because a FAIL that means "this machine" teaches a reader to stop
  reading.** `needs:<tool>` (ripgrep, PyYAML, `gradlew`) records NOT PROVEN when the
  tool is absent, which still fails the run — unproven is not verified, the same
  rule the target builds follow. `CI ONLY` is the one non-PASS verdict that does
  not fail the run, and the exception is closed by construction: it marks
  `test-local-verification-contract.sh`, which runs *this script* against fixture
  repositories and would otherwise nest the whole gate inside itself.

  **They run unconditionally, like the host suites.** Keying them to changed files
  would mean maintaining a file list per gate — a list to forget an entry from,
  which is how a gate stops covering what it names. Each is seconds. When T.3 makes
  the cargo suites selective, this lane is the one to leave alone.

  `--plan-only` reports the lane too: a plan that omits a whole lane is the same
  misreport this item exists to fix.

  **The lane's own first version under-ran silently, which is the failure mode it
  was built to prevent.** It iterated the derived list with `while read` over a
  here-string, and one gate runs `cargo`, which consumes stdin. That gate ate the
  rest of the list: three gates and the `CI ONLY` line were simply absent from a
  verdict block that still said "verified for every target this change touches".
  The list is now read into an array and every gate runs with stdin closed. The
  regression is pinned by a fixture whose first stub gate drains stdin and whose
  last one is checked for in the verdict -- reverting to the here-string loop fails
  that check by name, plus the two that assert a refusing gate fails the run.

  Contract additions, all in `scripts/test-local-verification-contract.sh`: the
  plan reports the lane; the lane contains the ad reward gate; the derivation
  yields at least fifteen gates; nothing the workflow's `quality-gate` job runs is
  absent from the lane (answered by the same parser through `--audit`, because a
  second grep would also match the artifact jobs and report their gates as missing
  forever); a gate after a stdin-draining one still reaches the verdict; and a gate
  that refuses fails the whole run. The fixture had to grow the product-profile
  features and a workflow with sixteen stub gates -- the real anti-vacuity floor is
  fifteen, and making that floor tunable so a fixture could pass with one gate
  would be a switch that turns the check off in production too.

  Verification for this item and for everything else in the working tree, one run,
  `scripts/verify-change.sh --base HEAD` over 29 changed files: **41 verdict lines,
  40 PASS and one `CI ONLY`, "verified for every target this change touches"**.
  The lanes: 16 host steps (14 Full plus the two Slim suites task 0.15 added), 24
  contract gates (23 run here, `test-local-verification-contract.sh` recorded
  `CI ONLY` because it runs this script against fixture repositories), and
  `android-java compile` running both product flavours. Counts inside those steps:
  shared 424, runtime-v8 Full 522 and Slim 471, graphics 571, io 266, capi 143,
  audio 65, core Full 62 and Slim 59, platform 52; Java 126 Full and 126 Slim.
  No Android native compile was required and none is claimed -- nothing in this
  change touches `#[cfg(target_os = "android")]` Rust, and the selector's plan says
  so out loud.




