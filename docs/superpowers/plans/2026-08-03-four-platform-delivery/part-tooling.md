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

  **The permission cluster was re-measured on 2026-08-09 and the "equivalent" verdict
  now rests on the source lines rather than on the classification.** A focused run over
  `PermissionOperationGate*` and `PermissionRevocation*` (93 mutations, 73 killed, 2
  timed out, 13 uncovered) leaves **5 survivors, all on `PermissionOperationGate`, and
  every one lands on a line that already returns that constant**:

  | Survivor | Source line | Mutator |
  |---|---|---|
  | `register:233` | `if (session.lifecycle != Lifecycle.ACTIVE) return null;` | `NullReturnVals` |
  | `register:235` | `if (entry == null \|\| !entry.granted) return null;` | `NullReturnVals` |
  | `enter:252` | `return false;` | `BooleanFalseReturnVals` |
  | `enter:255` | `return true;` | `BooleanTrueReturnVals` |
  | `runIfGranted:271` | `if (entry == null \|\| !entry.granted) return false;` | `BooleanFalseReturnVals` |

  Replacing a constant return with the same constant is the same bytecode, so no test
  can distinguish it. `PermissionRevocation` has **no** survivors. Anything written
  against these five would be a test that cannot fail, which is the thing this
  project's bar forbids — so the cluster is closed, not deferred.
  Note for a later reader: `--offline` cannot run pitest, because the
  `org.pitest:*:1.16.1` JARs are in their own `pitestRuntime` configuration and are
  not in the offline cache. Run this task without `--offline`.


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

  **A gate in the derived list was reading a build artifact nobody produced,
  found 2026-08-09 by a clean-tree run.** `scripts/test-android-host-api-contract.sh`
  reads `javap` output from
  `platforms/android/library/build/intermediates/javac/fullDebug/classes` and did
  not compile it. In CI that was accidentally safe: the workflow step is two lines,
  `./gradlew :library:compileFullDebugJavaWithJavac` and then the script. The
  derived lane lifts the `bash scripts/...` line alone, so locally the gate had
  **both** failure modes — FAIL on a cold tree for a reason unrelated to the
  change, and, worse, **PASS against stale bytecode**.

  `ci_contract_gates.py`'s `NEEDS` entry even said so: *"These compile the Java
  half themselves before reading it"*, a comment covering two entries where only
  `test-camera-frame-jni-contract.sh` did. The second inherited a reason that never
  applied to it — the same shape as item 0.6's `held_button_`, where one comment
  covered two fields.

  Fixed by making the gate compile what it reads, the way the camera gate already
  does, which also makes `verify-change.sh`'s claim that gates run "with the exact
  command line CI uses" true for this one. Evidence, with the pre-change script
  kept as the comparison scope:

  | | Classes deleted | Public method added to `MigoRuntime`, no rebuild |
  |---|---|---|
  | Pre-change script | FAIL "compiled classes not found" | **PASS**, "357 entries unchanged" — the false green |
  | After | PASS, compiles then reads | FAIL, printing the added `mutantHostApiMethod` |

  Restored by `git checkout` of the Java file, which is safe here because it was
  committed; the Rust mutants in this session were not, and were restored from
  copies.

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





- [ ] T.7 Make an unrun test binary fail the verifier. **Implemented and pinned;
  neither independent review has run, so the item stays open.**

  **Thirteen integration-test binaries holding 95 tests were run by no local step,
  and 35 of them by no job anywhere.** The cause was uniform and invisible to every
  check that existed: each gate names its suites per crate, and each one said
  `--lib`. `cargo test -p <crate> --lib` runs the lib's unit tests and *none* of that
  crate's `tests/*.rs` binaries, so a binary could exist, compile on every run, and
  never execute.

  Found while closing task 0.15, from the opposite direction: A6 names an "ABI and
  header contract" suite, and the recorded reason it had not been run was that it
  "needs the C package, which is a target build rather than a host suite". That is
  false. `migo-capi-abi` has no dependencies and no features at all -- it was split
  out of `capi` precisely so the boundary rules would be provable without a device or
  a graphics stack -- and its 60 tests run on the host in 0.01s. The real defect was
  that **the local verifier ran that crate not at all**, while CI ran it with
  `--all-targets`.

  The breakdown, measured:

  | Crate | Binaries | Tests | Ran locally | Ran in CI |
  |-------|---------:|------:|-------------|-----------|
  | `migo-capi-abi` | 9 | 60 | no | yes |
  | `migo-graphics` | 5 | 33 | no | no |
  | `migo-runtime-v8` | 2 | 2 | no | no |
  | `migo-shared` | 2 | 4 | yes | no |

  The graphics five are golden-image and decode tests and need no GPU -- Skia
  rasterises to memory -- and the whole set costs under two seconds.
  `shared/tests/frame_cycle_allocation.rs` is Section 7.3's allocation gate at the
  frame boundary and states its own reason for being a separate binary: a
  `#[global_allocator]` is unique per binary, and the command pools must be
  uncontended to measure a cycle at zero. CI never ran it.

  **Why the existing contract could not see this.**
  `scripts/test-local-verification-contract.sh` compared the two crate lists, and
  compared them one way: `local ⊆ CI`. That is the harmless direction -- a local step
  CI lacks makes CI narrower than a developer's machine. The direction that makes the
  *local verdict false* is `CI ⊆ local`, and it was unasserted, so `migo-capi-abi`
  passed every check in that file. Both directions are asserted now. Worse, a
  crate-name comparison cannot see scope at all: two lists naming the same crates run
  different binaries when one says `--lib`, and adding `test -p migo-capi-abi --lib`
  would have satisfied a name check while running **zero** of the 60 tests.

  **The fix is an audit, not a longer list.** `scripts/lib/host_test_coverage.py`
  asks `cargo metadata` for every `kind: ["test"]` target and reports the ones no host
  step runs; `verify-change.sh` runs it at startup and **refuses to produce a
  verdict** when the list is non-empty. Same argument as the module-walk audit
  directly above it: an unreached source file has unknown conditions, and an unrun
  test binary has unknown behaviour. `cargo metadata` is the authority on purpose --
  globbing `tests/*.rs` is a second implementation of cargo's target discovery that
  counts `tests/common/mod.rs` as a binary and cannot see an explicit `[[test]]`.
  A compile is deliberately not coverage: `build --workspace --all-targets` builds
  every one of these binaries and runs none, which is how they stayed invisible in a
  green tree.

  `--list-host-steps` was added beside `--list-host-crates` so the contract can see
  scope rather than re-derive it with a regular expression that has its own opinion of
  cargo's syntax. The same parser then audits CI's own `cargo test` lines, so the two
  sides are held at one scope by one implementation.

  Mutation evidence, four mutants, each showing the new scope fails while the scope it
  replaced stays green -- the second half matters, because a kill that the old scope
  also catches would not justify widening anything:

  - **M-T7-1** `validate_header` stops checking the ABI version. `--lib`: **0 passed,
    0 failed** -- the step passes with the defect live, which is what "a gate that
    cannot fail" looks like when measured. `--all-targets`:
    `foreign_abi_version_is_rejected_before_size` FAILED.
  - **M-T7-2** `99_main.js` deletes `globalThis.Deno` again, the historical
    snapshot defect. lib 522 passed; `snapshot_roundtrip_restores_deno_core` FAILED
    with `ReferenceError: Deno is not defined`, the original symptom.
  - **M-T7-3** `ClearRect` erases one pixel to the right. lib 571 passed;
    `clear_rect_erases_content_to_transparent` FAILED at "the rect's first column is
    erased". **This mutant survived the first time**: the test sampled one interior
    pixel and one far corner, both of which a one-pixel shift preserves. It is now
    asserted at all four boundaries, and the fixed test kills it.
  - **M-T7-4** the frame packet's op vector starts from `Vec::new()` again. lib 424
    passed, including all five `command_vec_pool` tests;
    `a_steady_state_frame_never_reaches_the_heap` FAILED with `64 fresh, 64 resize,
    43008 bytes`.

  The audit itself is pinned three ways in the contract: a fixture crate gains a
  `tests/orphan.rs` and the run fails naming `migo-demo::orphan`; the parser is asked
  directly whether a `--all-targets` *build* counts as coverage and must say no; and
  CI's own commands must leave no binary unrun. Its first version also proved it fails
  closed for the right reason -- the fixture helper did not copy the new script, and
  every fixture run refused with "cannot tell which test binaries the host steps run"
  rather than passing vacuously.

  Residual, recorded and not fixed here: `crates/io/src/image_cache.rs:1532` carries a
  duplicated `#[test]` attribute, and `ImageCache::clear` is dead. Both are
  pre-existing and unrelated to this item.

  **The widened gate found one thing immediately, and it was this change's own.**
  `scripts/test-r8-profile-contract.sh` step 5 compiles
  `platform/src/android/jni/profile_contract.rs` standalone with bare `rustc --test`
  and no `--cfg` at all, so the crate root never participates and the
  `compile_error!` added to `platform/src/lib.rs` — which makes a build with neither
  profile feature impossible — could not fire there. Item 0.15's new test then failed
  in the full gate: with no features, `active_methods` contributes nothing while the
  rule it is compared against says Full. Every *cargo* dependent had been checked and
  every one forwards a profile; the harness that does not use cargo had not been.
  Fixed by giving the standalone compilation each profile's feature set, one `rustc`
  run per profile, rather than by weakening the test — a profile-less compilation
  modelled no shipped product, which is why the gap existed. Verified load-bearing:
  deleting `#[cfg(feature = "api-system")]` and its `extend_from_slice` now fails that
  script under `profile-full`.

  **Verification for this item and the whole working tree, one run,
  `scripts/verify-change.sh --base master` over 15 changed files: 47 verdict lines,
  44 PASS, one `CI ONLY`, two `NOT PROVEN`.**

  The `CI ONLY` is `test-local-verification-contract.sh`, which runs this script
  against fixture repositories. The two `NOT PROVEN` are **`ohos compile` and
  `windows compile`, and they are toolchain-blocked on this machine, not skipped**:
  the selector demands them because `engine/crates/platform/src/lib.rs` changed and
  that file is the crate root declaring `#[cfg(target_env = "ohos")] pub mod ohos`
  and `#[cfg(target_os = "windows")] pub mod windows`. Neither toolchain exists here.

  What was done instead of compiling them, since the change to that file is a
  `compile_error!` and the risk is precisely that it fires on a target build:
  both target builds were read for the features they pass. `build-ohos-host.sh:61`
  and `build-ohos-sdk.sh:116` both pass `--no-default-features --features
  profile-slim`; `build-windows-sdk.sh:162` runs `cargo build -p migo-capi --release`
  with no `--no-default-features`, so `capi`'s own `default = ["profile-full"]`
  forwards `platform/profile-full`. Both therefore satisfy the new requirement. That
  is a static argument, not a compile, and it is recorded as such.

  Lanes and counts: 18 host steps (the four new ones among them), 24 contract gates
  (23 run here), `android compile` for `arm64-v8a` and `android-java compile` over
  both product flavours. 43 host suite results, zero failures. Inside them:
  capi-abi 60 across nine binaries, shared 424 + 4 integration, runtime-v8 Full 522
  + 2 integration and Slim 471 + 1, graphics 571 + 33 integration, io 266, audio 65,
  core Full 62 and Slim 59, capi Full 143 and Slim 143, platform Full 53 and Slim 53.
  The Qt host kit is not part of this run and was verified separately for item 0.6.

- [ ] T.8 Make the OpenHarmony compile a target the verifier knows about. **T.1 for a
  third platform, and it existed only because a comment in the verifier was believed
  instead of checked. Implemented and pinned; neither independent review has run.**

  `verify-change.sh` said, in the header above its target lanes, that "`ohos` and
  `windows` conditional code has no local build on this machine". Half of it was
  false. The OpenHarmony SDK is at `~/ohos-sdk` and `scripts/dev-setup-ohos.sh`
  resolves it, `x86_64-unknown-linux-ohos` is an installed Rust target, the prebuilt
  V8 archive for the triple is in tree, and `scripts/build-ohos-sdk.sh` has been
  running that cargo build for months. **The compile takes 13 seconds warm.** So
  every change touching `cfg`-conditional Linux code — including the X11 work in
  item 0.4 — collected a permanent `NOT PROVEN` that could have been evidence.

  **How invisible that lane's subject was.** `cargo`'s own dep-info says it: the
  Android build of `migo-capi` lists `crates/capi/src/platform/{android,mod}.rs`,
  the OpenHarmony build lists `{mod,ohos}.rs`. `ohos.rs` is not merely untested by
  the other lanes — it is not a dependency of them, so touching it does not even
  invalidate their fingerprints. Before this item, nothing on this machine compiled
  that file.

  **The lane calls `build-ohos-sdk.sh --compile-only x86_64` rather than restating
  its cargo line.** The `dev-setup-ohos.sh` exports and `RUSTY_V8_ARCHIVE` are
  exactly the kind of rule a second copy gets silently wrong: without the pins, a
  machine with an Android NDK on `PATH` compiles the C dependencies with the NDK's
  clang and bionic headers for a musl target. `x86_64` because the lane proves the
  `target_env = "ohos"` view of the tree compiles, which is the same view for either
  architecture, and it is the warm one.

  `--compile-only` returns before the package contract and the API floor gate, and
  that is load-bearing rather than a shortcut: both read `dist/migo-ohos-<arch>`,
  which the mode does not write, so running them would gate whichever package an
  earlier full run left behind. Writing it the other way reproduced, inside this
  same session, the stale-artifact defect being fixed in T.6.

  **Probed, not assumed**, like `android-java`: absent SDK or absent V8 archive
  records `NOT PROVEN`, not a `FAIL` that blames the change for a machine.

  **Mutation evidence.** Removing `MIGO_ERROR_UNSUPPORTED_PLATFORM` from the imports
  of `capi/src/platform/ohos.rs`:

  | Scope | Result |
  |---|---|
  | `bash scripts/build-ohos-sdk.sh --compile-only x86_64` | **FAIL**, `error[E0425]: cannot find value MIGO_ERROR_UNSUPPORTED_PLATFORM` |
  | `cargo test -p migo-capi --lib` | 147 passed |
  | `bash scripts/build-android-so.sh --compile-only arm64-v8a` | SUCCESS |

  The two passing scopes did not merely tolerate the mutant, they never read the
  file — which is the strongest form the claim can take. Restored from a copy and
  verified by `sha256sum`.

  **Still absent, and correctly so:** `windows`. Its toolchain is on the other side
  of the WSL boundary and there is no local lane, so it stays `NOT PROVEN`.

  Verified 2026-08-09 in the run recorded under item 0.4: 19 host steps, 23 contract
  gates, `android compile` PASS and `ohos compile` **PASS** where the same change had
  reported `NOT PROVEN` an hour earlier. `scripts/test-local-verification-contract.sh`
  was also run directly — it is `CI ONLY` from inside the verifier because it would
  nest, and this item changes the verifier — and reports all checks passed.
