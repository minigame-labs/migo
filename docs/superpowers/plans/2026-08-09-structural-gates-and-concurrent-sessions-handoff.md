# Structural Gates and Concurrent Sessions — Handoff, 2026-08-09 (later)

**Bootstrap on another machine:** read
`docs/superpowers/plans/2026-08-08-four-platform-delivery-handoff.md` §0 first. It
lists the git-ignored prerequisites — Android V8 archives via
`scripts/fetch-v8-archives.sh`, a host linux-gnu V8, and
`export MIGO_HOST_V8_DIR=$PWD/engine/third_party/rusty_v8/x86_64-linux-gnu/gn_out`
in every shell. Without them every host suite fails for a reason unrelated to your
change. Nothing there has changed.

**Branch:** `gate/runtime-generation-fence`, seven commits on top of `origin/master`
(`3196c3a`), **unpushed**. Verify before believing: `git log --oneline 3196c3a..HEAD`.

**Ledger:** `docs/superpowers/plans/2026-08-03-four-platform-delivery/part-phase-0.md`
— item 0.9's task-7 entries (the fence gate, "fifth") and item 0.68 (entries 1, 2
and 3). Everything below is the short form.

---

## 1. What landed

| commit | what |
|---|---|
| `514a523` | `test-runtime-generation-fence-contract.sh` — the fence checked structurally, wired into `pr-ci.yml` |
| `7c67e70` | `/tmp` is per session on both sides, with a sweep of the directories no live session owns |
| `6df2bad` | three JNI exports take the calling session's id; `RuntimeRegistry.getAny()` and `clear()` deleted |
| `e0824e1` | `test-jni-outbound-signature-contract.sh` — every `JAVA_*` descriptor against its Java declaration |
| `33522fb` | the ledger and this document |
| `10c6331` | the log level is per session, in three tiers, on both sides |

---

## 2. Two claims in the previous documents were false, and both mattered

**The fence gate's proposed derivation rule was wrong.** The 2026-08-09 handoff said
"parse `profile_contract.rs` for every `NATIVE_*` descriptor matching `^\(IJ` — that
is exactly *this callback carries a runtime generation*". It over-selects by three of
seventeen: `onVsync(IJ)V`, `setDisplayRefreshRate(IJ)V` and
`getConsoleLogs(IJ)Ljava/lang/String;` put a *payload* long in that slot — a frame
timestamp, a refresh period, a log cursor. A gate on that rule would demand
`token.generation()` where a frame timestamp belongs, and the only way to satisfy it
would be to break vsync. The authority is the JNI handler's own parameter list in
`inbound.rs`: `host_id: jint, generation: jlong`. Fourteen handlers declare it; all
fourteen convert it with `captured_generation(generation)`.

**Item 0.68's "the profile contract is what will catch a half-done one" was false.**
It compares method *names*. Measured: widening `getSystemSettingInfoBytes` to `(I)[B`
in Rust while leaving the Java method no-arg passes the product profile contract, both
R8 root checks, the Android host-API contract and `javac`. Nothing caught it until
`GetStaticMethodID` failed on a device. That is what `e0824e1` exists for.

The lesson is not "those documents were careless". It is that a claim of the form "X
will catch it" is testable in about two minutes by breaking the thing and running X,
and neither claim had been.

---

## 3. The two gates, and how to extend them

Both live in `pr-ci.yml`'s `quality-gate` and are therefore derived into
`verify-change.sh`'s contract lane by `scripts/lib/ci_contract_gates.py`. Both read
their inputs through `scripts/lib/jni_source.py` — maskers that blank comments and
literals while preserving offsets, bracket matching, argument splitting, the
descriptor tables, and a JNI descriptor decoder.

**If you add a masker fix, it lands in one place.** That was the point of extracting
it; the fence gate lost 169 duplicated lines. The Rust masker handles what
`inbound.rs` actually contains: 137 lifetime annotations (a naive char-literal rule
blanks the parameter list behind each one) and 11 raw strings.

**`test-runtime-generation-fence-contract.sh`** derives the fenced set from three
engine facts that must agree — the handler's `generation: jlong` parameter, its
`captured_generation` conversion, and a descriptor beginning `(IJ` — then requires
every Java call site to stamp a `final RuntimeGenerationBoundary.Token` field *of the
same file* or `RuntimeGenerationBoundary.UNFENCED`. The `final` requirement is the
whole re-read check: a final field can only have been assigned once, at construction.

It also reads the descriptor set backwards: `(IJ` is the entire surface a generation
can arrive on, so every method there must be a handler the parse can *see*.
`inbound.rs` generates handlers from `jni_json_callback!`, and one generated that way
is invisible to a text parse — without that check the fenced set could shrink by one
silently, which is the failure the gate exists to prevent.

**`test-jni-outbound-signature-contract.sh`** decodes all 126 `JAVA_*` descriptors and
matches each against a `public static` declaration in `NativeExports`. Reference types
compare by simple name. Only `public static` answers for a descriptor.

---

## 4. State of item 0.68

Items 1, 2 and 3 are closed, and item 8's `RuntimeRegistry.clear()` went with item 2.

Item 3's shape is worth knowing before touching the next one, because the same
question recurs: a setting that arrives *per session* meeting a resource that is
*process-wide*. `shared::log_level` answers it in three tiers — the thread's session
(exact, because a host thread can be attributed), the join over live sessions (for
threads that cannot), and the process default. Item 5 below is the same shape and
should probably get the same answer.

Remaining, in the ledger's order:

4. **Image decode has no per-session partition** (`io/src/image_ops.rs:83-84` `SEM`,
   three permits; `:160-161` `BUDGET`, 48 MB) while the IO executor next door does
   per-host fair queuing. Latency, not correctness.
5. Diagnostics attributed to the process rather than the session
   (`shared/src/render_command_sender.rs:58,82` summed into each session's
   `command_drops`). Host-app visible only.
6. `graphics/src/frame_capture.rs:21-22`, a last-presented-frame singleton written
   from every render thread.
7. `audio/src/streaming.rs:245`, one single-worker runtime for every session's
   streaming downloads, nothing draining it at session end.
8. Latent, no in-tree caller: `PACKAGE_SIGNATURE_VERIFIER`
   (`shared/src/vfs/package.rs:1288`), `isolate_pool.rs:41`.

**Bluetooth remains the only unfenced Android producer group**, deliberately — §4 of
the previous handoff has the whole design and the reason, and none of it has changed.

---

## 5. Things it would be easy to get wrong

**A fence and a cache sweep are one change.** Managers are cached per session, not per
runtime. Fencing a producer without routing its cache lookups through
`RuntimeGenerationBoundary.liveEntry` turns "events reach the wrong runtime" into
"events reach nothing at all".

**Whether a group may be swept at restart depends on its own teardown**, not on
whether it is fenced. A teardown that reports (camera emits `stop`, keyboard emits
`onKeyboardComplete`) puts those events on the queue while `on_restart` is still
running, so they reach the runtime that *replaces* this one.

**`HostCommand` sits exactly on its 64-byte cap.** New fenced variants use
`Option<NonZeroI64>` and `captured_generation`. Measured: reverting one media variant
to `Option<i64>` fails the build on the enum's own assertion.

**`OnDeviceOrientationChange` and network status are deliberately unfenced.** They are
current facts about the device; dropping them leaves a fresh runtime with a stale view
and nothing to correct it.

**`/tmp` is now per session, and the sweep's ordering is load-bearing.**
`GameSession` sweeps before creating its own directory, so its own id is not yet
registered and a directory left by a dead session holding that id is swept rather than
inherited. Move the sweep after `ensureDirectories()` and it deletes the directory the
session is about to use. Session creation is main-thread-only
(`ThreadCheck.ensureMainThread`), which is what makes reading `RuntimeRegistry` there
race-free.

**A runtime restart re-runs `evaluate_module`** (`core/src/runtime/host.rs:1715`). Any
"once per session" work put there runs again on every restart — which is why the temp
wipe lives in the Java `GameSession` constructor and not in the engine's evaluate
path.

---

## 6. Verifying, on a machine that is not this one

**Iterate narrowly.** Java: `cd platforms/android && ./gradlew --no-daemon
:library:testFullDebugUnitTest :library:testSlimDebugUnitTest`. Rust: the affected
crates' `--tests`. **Anything touching `cfg(target_os = "android")` must run
`bash scripts/build-android-so.sh --compile-only arm64-v8a`** — that is what caught
`jint` not being in scope in `outbound.rs` this session; the host lane cannot see that
file at all.

**`--offline` fails on a cold Gradle cache here**, with `No cached version of
org.json:json:20240303`, presented behind a `Configuration cache state could not be
cached` line. Run once without `--offline` to populate it.

**Read the test counts, not `BUILD SUCCESSFUL`.** A `--tests` filter that matches
nothing still prints success. Sum `tests`/`failures` out of
`library/build/test-results/<task>/*.xml`.

**The full gate is for handoffs:** `bash scripts/verify-change.sh --base <ref>`. It
now derives 26 contract gates.

**Windows reads NOT PROVEN without the MSVC toolchain**, and that is the honest
answer — do not infer it from the Linux build.

**Mutation harness rules, and two new ones.** A mutation that fails to apply must
abort loudly. Every `killed` line must name the test that failed. **A `TYPE SYSTEM`
verdict is the one to distrust**: a harness written this session reported all five
Rust mutants as caught by the compiler, because cargo prints `error: test failed`
after a red suite and the harness matched that as a compile error. Five of five
caught by the compiler is not credible for a `.min()` → `.max()` mutation, which is
what prompted checking — read test failures *before* compile errors. And: **an
equivalent mutant must be discarded, not counted.** One mutation this session reported a loose
temp file as owned by session `0`; since `0` is not live either, the file was still
deleted and the suite still passed. It was recorded as NOT DETECTED and replaced with
a mutation that changes behaviour — counting it would have invented a survivor.

Repo files are root-owned with writable directories, so a harness must write through
`os.replace` rather than in place, and its scratch copies must not use an extension
the gates scan (`*.java` is scanned; `*.java.harness-original` is not).
