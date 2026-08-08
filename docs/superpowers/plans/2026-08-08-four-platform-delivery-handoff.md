# Four-Platform Delivery — Handoff, 2026-08-08

**Authority:** `docs/superpowers/specs/2026-08-03-four-platform-delivery-design.md`.
**Ledger:** `docs/superpowers/plans/2026-08-03-four-platform-delivery.md` — the
status convention, and every closed item's evidence, lives there. Read its
"Status Convention" section before marking anything.

**Branch:** `perf/ble-notification-path`, 10 commits ahead of `master`.

> ⚠️ **The branch is local-only. Push it before this handoff is usable from
> another machine** — nothing below can be checked out otherwise.

---

## 0. Bootstrap on a different machine

This document was written on the machine the work was done on. If you are
elsewhere, the repository alone is not enough: several prerequisites are
git-ignored, sibling to the repository, or system-level.

**Everything below is required before `scripts/verify-change.sh` can pass.**
None of it is optional, and each one fails in a way that reads like something
else.

1. **The Android V8 archives are git-ignored.** They live in
   `engine/third_party/rusty_v8/{aarch64,x86_64}/` and are fetched from GitHub
   Release assets, not stored in git:
   ```bash
   bash scripts/fetch-v8-archives.sh
   ```
   Missing archives fail the Android build with a linker error, not a clear
   "download this" message. A `git reset --hard` also wipes them.

2. **Host builds need a linux-gnu V8 that this repository does not ship.** The
   in-tree archives are Android builds; linking them into a host binary fails
   with `incompatible with elf64-x86-64`. The host toolchain expects a sibling
   checkout of `rusty_v8` built for `x86_64-unknown-linux-gnu`, found by default
   at `../rusty_v8_src/target/x86_64-unknown-linux-gnu/release/gn_out/obj/`.
   Override with `MIGO_HOST_V8_DIR`. Without it, every host suite fails for a
   reason unrelated to any change — `verify-change.sh` says so out loud when it
   probes and cannot find the toolchain, and that message is the one to read
   first when a fresh machine reports a wall of red.

3. **Host Skia needs headers and libraries** that `scripts/dev-setup-skia.sh`
   installs into `~/.local` (Khronos headers, `libEGL`/fontconfig/freetype
   symlinks, ninja). `dev-test-host.sh` runs it, but system packages
   (`libasound2-dev` for the audio crate, a `clang`, `ninja-build`) may need a
   `sudo apt-get install`, which an agent cannot do — ask the human.

4. **Android SDK + the pinned NDK.** The NDK version is pinned and resolved by
   identity; a different one is rejected rather than silently used. `ANDROID_HOME`
   and the NDK path must be set. A JDK is needed for Gradle.

5. **Then the tools:**
   ```bash
   curl -L --proto '=https' --tlsv1.2 -sSf \
     https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
   cargo binstall -y cargo-mutants cargo-nextest hyperfine
   ```
   Prebuilt binaries deliberately: building them from source inside this
   repository hits the toolchain trap in §4. `pitest` needs no install — it is a
   Gradle plugin, and adding it is an open task (§3.3). **`sccache` is not needed
   yet**; it only matters for parallel worktrees (§5).

6. **Sanity check before doing any work**, so a broken environment is not
   mistaken for a broken change:
   ```bash
   bash scripts/verify-change.sh --base HEAD     # clean tree: everything should PASS
   ```

**Not portable at all:** the physical device (a Huawei Mate30 Pro), the
`migo-api26` x86_64 emulator, and the Windows/MSVC toolchain are properties of
the original machine. Any ledger item needing device evidence (0.65, 2.2, 2.5,
2.6, 5.2, 5.3) cannot be closed without equivalent hardware.

---

## 1. How to verify anything

**One entry point, and it is now honest about both languages:**

```bash
bash scripts/verify-change.sh --base master        # what a PR would gate
bash scripts/verify-change.sh --base HEAD          # the working tree alone
bash scripts/verify-change.sh --base master --plan-only   # what it would run, and why
```

It audits the module walk, asks `scripts/lib/verification_targets.py` which
targets the changed files need, runs the host suites, runs the target builds,
and prints one verdict line per target. A non-PASS line fails the run. **Copy
the verdict block into the ledger entry** — the specification requires every
change touching conditional code to name the target build that compiled it.

Individual suites:

```bash
bash scripts/dev-test-host.sh test -p migo-shared --lib
bash scripts/dev-test-host.sh test -p migo-core --lib
bash scripts/build-android-so.sh --compile-only arm64-v8a
cd platforms/android && ./gradlew :library:testFullDebugUnitTest :library:testSlimDebugUnitTest
```

`dev-test-host.sh` is not optional for host builds: it establishes the system
clang, the Khronos headers and the linux-gnu V8 archive that four Skia-linked
crates need. It passes any cargo subcommand through, so `cargo-mutants` runs
inside it too (§2).

⚠️ **Confirm `Compiling <crate>` really appears** in an Android build before
concluding a fix did or did not work. Under WSL2 cargo misses mtimes written by
scripts and happily hands back a stale artifact; `touch` the source to force it.

---

## 2. Mutation testing is now automated — use it

This project's quality bar is *"a guard that cannot fail is decoration"*, and it
has been enforced by hand-written mutants. `cargo-mutants` does it
automatically and reports **survivors**, which is exactly the artifact that
matters:

```bash
bash scripts/dev-test-host.sh mutants \
  --file crates/shared/src/payload_pool.rs --package migo-shared --timeout 120 -j 2
```

**Always scope with `--file`.** The whole workspace would generate thousands of
mutants, each a rebuild.

**Read the survivors, do not just run it.** Its first run here reported 13
survivors out of 25 in `payload_pool.rs`; the cause was that the new
`RecyclePool`'s tests all lived in a *different crate* (`migo-core`), so
`migo-shared`'s own suite never touched it. Adding tests where the mechanism
lives took it to **2 survivors, both `Debug::fmt`** — genuine noise, since
nothing asserts on debug output. Expect that shape: a handful of unkillable
mutants (Debug impls, `#[must_use]` returns nobody reads) are fine; anything
touching a bound, a comparison, or a `Drop` is not.

**The tool does not replace the thinking.** It tells you which mutants nobody
kills; deciding whether a survivor is noise or a hole is still yours.

---

## 3. What to do next, in order

### 3.1 Finish A12 (ledger task 0.12) — two of its three clauses

A12 reads: *"reject zero, negative, non-finite, and otherwise invalid host pixel
ratios; canonicalize Windows game identity with the same rules as the other
platforms; and make a missing ad handler settle the content-visible request
through its documented error path rather than leaving it pending."*

- **Pixel ratios: already done.** Verified 2026-08-08. `PixelRatio::new`
  (`engine/crates/shared/src/surface/geometry.rs:13`) requires finite and
  positive; `engine/crates/capi-abi/src/surface.rs` rejects invalid scale
  factors at the ABI boundary, with
  `generation_dimensions_and_scale_are_strictly_validated` iterating
  `[0.0, -1.0, NaN, INFINITY, -INFINITY]`. Every construction site validates and
  none has an `unwrap_or` fallback. **This clause is stale in the ledger and
  needs only a correction, not work.**

- **Missing ad handler: investigated 2026-08-08. The defect is live, but not
  where A12's wording suggests.** Findings, all with evidence in the source:

  - **Android is unconditionally "hosted".**
    `engine/crates/platform/src/android/services/mod.rs:204` returns
    `Some(AndroidAd)` gated only by the `api-commerce` cargo feature, never by
    handler registration, so `op_ad_is_supported` is `true` for every full-profile
    Java-SDK session and the JS takes the hosted path with no local fallback.
    Slim (feature off) cfg-deletes it and gets the correct no-host path.
  - **Three of six ad entry points settle; three return silently.**
    `createAd`/`loadAd`/`showAd` go through
    `NativeExports.adHandlerOrReportError` (~`:2810`), which emits an `error`
    with `"<api>:fail no ad handler"`. `hideAd`/`updateStyle`/`destroyAd`
    (~`:2899`, `:2915`, `:2934`) do a bare `sAdHandlers.get` and `return`. Only
    one of those three leaves anything content-visible pending: `hideAd`, for
    `CustomAd`, whose `onHide` fires on the no-host path and never on
    hosted-without-handler. The other two owe content no event. The helper's own
    javadoc states the rule its siblings break — *"a silent drop leaves it
    waiting forever"*.
  - **The larger defect is a contract mismatch, not the silent return.**
    `platforms/android/library/src/main/java/com/migo/runtime/callback/AdHandler.java:11-16`
    documents the no-handler path as *"incentivised video closes with
    `isEnded = false`"* — which is the **no-service** contract, not the
    **service-installed-but-no-handler** one Android always takes. A rewarded
    video `show()` with no handler emits `error` and **never `close`**, while
    `show()` resolves. Content following the wx idiom (`onClose` decides the
    payout) waits forever, and the SDK's own documentation promised otherwise.
  - **C ABI, Linux, Windows and OHOS have no defect here.** None installs an
    `AdService` (`capi/src/host_kit.rs:214`, `platform/src/{linux,windows}/platform.rs`
    empty `CommerceServices` impls; OHOS has no `DeviceServices` at all), so the
    correct no-host JS path runs. Their gap is Section 3.4 *parity* — no ad
    capability exists to register — which is task 3.4 work, not A12.

  **The fix belongs in the Java adapter, and two existing tests prove it cannot
  go anywhere else.** A JS-side timeout is *prohibited*:
  `ad_reward_integrity.rs:488 hosted_ads_do_not_self_close_on_a_timer` and
  `:503 hosted_ads_do_not_self_report_load_on_a_timer` boot a service that
  records without emitting and assert zero `close`/`load` after ten seconds —
  correctly, because a slow real host would otherwise be raced by a fabricated
  close, which is the reward-integrity invariant. An op-layer fix is impossible:
  `ad_command_op!` knows only whether a *service* exists; handler registration is
  Java-side state. The adapter is the only layer holding both facts.

  So: route the three silent entry points through `adHandlerOrReportError`; then
  decide **one** of — correct `AdHandler`'s javadoc to say every call settles as
  `error`, or have the Java adapter synthesise `close{isEnded:false}` (it may:
  it knows no advert was shown, and it is the host side of the reward boundary,
  so the runtime's "never mint a reward" invariant is untouched). Today the code
  and the documentation disagree and only the documentation is untested.

  **No test catches any of this today.** `ad_reward_integrity.rs` covers
  *service returns `Err`* (a different mechanism) and asserts the pending state
  is correct at the JS layer; the contract script is source-level only; and
  **there are no Java ad tests at all** — 23 test files under
  `platforms/android/library/src/test`, none referencing `AdHandler`. Pin it with
  (a) a Java unit test iterating **the entry-point set** rather than a hardcoded
  three, so a seventh ad op is covered, and (b) a Rust test that a hosted
  `CustomAd.hide()` asks the host rather than self-firing `onHide`.

- **Windows game identity: investigation dispatched but not returned.** Start
  from `HostJsRuntime::evaluate_module` building
  `GamePaths::new(files_dir, cache_dir, game_id)`
  (`engine/crates/runtime-v8/src/host_runtime.rs`, ~831 and ~942), which is what
  `crate::storage::storage_dir` resolves against. The question is whether the
  shared canonicaliser is sufficient on Windows: case-insensitivity, backslash
  separators, reserved device names (`CON`, `NUL`, `COM1`…), trailing dots and
  spaces silently stripped by the filesystem, and alternate data streams
  (`name:stream`) can each make two distinct game ids collide on one directory —
  which would breach per-game storage isolation, a shipped invariant.

### 3.2 Make the host suites selective (speed, ~1 session)

`scripts/verify-change.sh` runs **all** host cargo suites on every invocation —
its own header says so at line 20: *"runs the host suites, always"*. A Java-only
change therefore pays eleven Rust suites. `scripts/lib/verification_targets.py`
already maps changed files to crates (`_CRATE_PATH`), so the missing piece is a
reverse-dependency closure from `cargo metadata`: run the changed crate's suite
plus every crate that depends on it.

**Get this right or not at all** — under-running is a silent gap, which is
strictly worse than slow. `scripts/test-local-verification-contract.sh` must
grow an assertion that a change in a leaf crate still runs its dependents.

### 3.3 Add pitest for the Java half

Java mutation testing is hand-rolled today. The Gradle plugin
(`info.solidsoft.pitest`) would do for `platforms/android/library` what
cargo-mutants does for the Rust crates. Scope it to
`com.migo.runtime.internal.*` and start with the BLE and permission classes,
which are where the concurrency invariants live.

### 3.4 Split the ledger

`docs/superpowers/plans/2026-08-03-four-platform-delivery.md` is ~5,500 lines.
It burns context on every read and is a merge-conflict magnet — it is the single
biggest obstacle to running several agents at once. Split it per phase with a
stable index, keeping item identifiers (`0.67`, `1.1a`, …) unchanged, because
other documents and commit messages cite them.

---

## 4. Traps that cost real time here

- **The NDK poisons host builds.** Global `CC`/`CXX` point at the NDK clang and
  `engine/.cargo/config.toml` has an `[env]` block, so `cargo install` of
  anything with C dependencies fails from inside the repo — that is why
  `sccache` would not build. Use prebuilt binaries, or
  `CC=/usr/bin/clang CXX=/usr/bin/clang++` with `ANDROID_NDK*` unset.
- **Do not restore mutants with `git checkout`.** The work being mutated is
  usually uncommitted. Copy the pristine files aside and restore from the copy;
  verify with `sha256sum`.
- **The six V8 snapshots are stale on `master`**, from commits #26–#29, not from
  this branch. One regeneration round covers the whole batch and belongs
  **last**: the fingerprint includes every `runtime-v8` `.rs` file and
  `engine/Cargo.lock`, so a `cargo fmt` or one added test invalidates it again.
- **A recorded obstacle in this ledger has been wrong eight times.** When an
  item says something is impossible, verify it against the code before believing
  it. The question that has dissolved most of them is *"which layer can see this
  property?"* rather than *"how do I reach this code?"*.
- **Guards cover the side they were designed for.** Twice in one session a new
  guard passed its own mutant while the defect it existed for survived. After
  writing one, ask what it does *not* cover, and write the mutant that exploits
  that.

---

## 5. On running several agents at once

Parallelise **investigation**, not builds. Every cargo build contends on one
target directory and one Skia/V8 environment; a fresh git worktree makes it
worse, because it has no `engine/target` and would rebuild Skia (~1,469 ninja
steps), and `dev-test-host.sh`'s default V8 path is relative to the repository
root, so it resolves wrongly from a worktree.

If you do run worktrees: share one `CARGO_TARGET_DIR`, set `MIGO_HOST_V8_DIR` to
an absolute path, fetch the git-ignored V8 archives with
`scripts/fetch-v8-archives.sh`, install `sccache`, and split the ledger first
(§3.4). Partition by crate ownership so no two agents touch the same crate, and
keep design decisions central — a cold agent is exactly the reader most likely
to believe a wrong recorded obstacle.

---

## 6. Closing an item

The ledger's own convention: `- [x]` requires implementation, behavioural tests,
fresh verification output, an independent spec review **and** an independent
code-quality review. Tasks 0.24 and 0.67 are implemented, tested, mutation-
verified and recorded, and are deliberately left `- [ ]` because neither review
has run. `/code-review ultra` is user-triggered and billed; batch four to six
items into one round rather than reviewing each alone.
