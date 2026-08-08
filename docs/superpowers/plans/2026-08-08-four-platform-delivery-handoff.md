# Four-Platform Delivery — Handoff, 2026-08-08

**Authority:** `docs/superpowers/specs/2026-08-03-four-platform-delivery-design.md`.
**Ledger:** `docs/superpowers/plans/2026-08-03-four-platform-delivery.md` — the
status convention, and every closed item's evidence, lives there. Read its
"Status Convention" section before marking anything.

**Branch, corrected 2026-08-09.** `delivery/a12-and-verification-lanes` no longer
exists: it was squash-merged as `079e954` (#31) and is on `origin/master`. Do not
commit to `master`; branch from it. The work after that merge is on
`delivery/x11-and-mutation-evidence`.

If a handoff tells you a branch is unpushed, check `git rev-parse` on the commit
it names before believing it — this one had already landed.

**Every shell needs this**, or every host suite fails for a reason unrelated to
your change (see §0.2):

```bash
export MIGO_HOST_V8_DIR=$PWD/engine/third_party/rusty_v8/x86_64-linux-gnu/gn_out
```

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

2. **Host builds need a linux-gnu V8, and it is a fetch rather than a sibling
   build.** The `aarch64`/`x86_64` in-tree archives are Android builds; linking
   one into a host binary fails with `incompatible with elf64-x86-64`. The
   linux-gnu archive is a *third* release asset and is fetched the same way:
   ```bash
   bash scripts/fetch-v8-archives.sh x86_64-linux-gnu
   ```
   That writes `engine/third_party/rusty_v8/x86_64-linux-gnu/librusty_v8.a`, which
   is **not** the layout `dev-test-host.sh` expects — it wants
   `$MIGO_HOST_V8_DIR/obj/librusty_v8.a` plus `$MIGO_HOST_V8_DIR/src_binding.rs`.
   Build that layout once, with **hard links, not symlinks**: the script's
   LFS-pointer check uses `stat -c %s`, which reports a symlink's own size and
   rejects it as a pointer file.
   ```bash
   D=engine/third_party/rusty_v8/x86_64-linux-gnu
   mkdir -p $D/gn_out/obj
   ln -f $D/librusty_v8.a $D/gn_out/obj/librusty_v8.a
   ln -f $D/src_binding.rs $D/gn_out/src_binding.rs
   export MIGO_HOST_V8_DIR=$PWD/$D/gn_out    # gitignored, costs no disk
   ```
   The default path (`../rusty_v8_src/target/x86_64-unknown-linux-gnu/...`) needs a
   source build of V8 and does not exist on this machine. Without either, every
   host suite fails for a reason unrelated to any change — `verify-change.sh` says
   so out loud, and that message is the one to read first when a fresh machine
   reports a wall of red.

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
   Gradle plugin, and adding it is an open task (§3.4). **`sccache` is not needed
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

**Two lanes `verify-change.sh` does not cover, and both matter:**

```bash
bash scripts/test-linux-qt-host-kit.sh     # Qt 6 host kit: 22 input + 13 session tests, ~1 min
```

The Qt host kit links a fake C ABI and builds neither the engine nor V8, so it is
outside the verifier entirely and has to be run by hand when
`platforms/linux/host-kit/**` changes. It builds in a temp dir and cleans up. It
runs each suite twice — once offscreen, where `xcb`-only tests report `SKIP`, and
once under xcb where they run — so **check for `PASS` on the test you care about,
not just for a zero exit**. It compiles with `-Werror=unused-function`.

The second is CI: `.github/workflows/pr-ci.yml` and `release.yml` are the merge and
tag gates, and `scripts/test-local-verification-contract.sh` now audits their
`cargo test` lines with the same parser the local gate uses. A change to the local
step list that is not mirrored in CI fails that contract.

⚠️ **Confirm the crate really recompiled** before concluding a mutant survived.
Under WSL2 cargo misses mtimes written by scripts — a `mv`-based write is not
enough either — so `touch` the source after mutating. And grep the build log for
the **package** name: cargo prints `Compiling migo-graphics`, not
`Compiling graphics`, and one wasted round here came from grepping the lib name and
reading "no recompile" when it had recompiled fine.

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
mutants, each a rebuild. Also pass `--output /tmp/<somewhere>`: the default
`mutants.out` goes next to the workspace manifest, `engine/` is root-owned, and
the run dies on `lock.json` with a bare `Permission denied`.

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

Rewritten 2026-08-08 at the end of the second session. Everything the earlier
version of this section listed as "next" is now either done and recorded in the
ledger, or corrected below because its premise was wrong.

### 3.1 Done and recorded this session

Read the ledger entry rather than redoing any of it. Each is implemented, has
mutation evidence, and is deliberately left `- [ ]` because both independent
reviews are user-triggered (§6).

- **Task T.7 — an unrun test binary now fails the verifier.** Thirteen
  integration-test binaries holding 95 tests were run by no local step, and 35 of
  them by no job anywhere, because every gate on both sides said `--lib`.
  `scripts/lib/host_test_coverage.py` asks `cargo metadata` for every
  `kind: ["test"]` target and `verify-change.sh` refuses to print a verdict when a
  step covers none of them. Four mutants, each showing the new scope fails while the
  scope it replaced stays green.
- **Item 0.15 (A6) — the remaining suites run.** The two recorded obstacles were
  both false: the ABI and header suites need no C package (`migo-capi-abi` has no
  dependencies and no features; 60 host tests in 0.01s), and the comment saying
  "`capi` and `platform` do not build on the host at all" sat four lines below the two
  steps that build and test them. The lifecycle, reattachment and input-saturation
  suites A6 names are `migo-capi` lib tests, now run under both profiles.
- **Item 0.6 (A5) — a shipped defect fixed.** The Qt view cached the held mouse
  button and never cleared it, so every hover after a right-click told content the
  secondary button was down. Fixed by deleting the cache and asking the event.
- **Items 0.2 (A1) and 0.3 (A2) — audited, found already implemented.** Their plans'
  unchecked steps carry "Expected: FAIL because…" premises that are all false. The
  ledger entries name the file and line for each, and the tests that already assert
  the properties.

### 3.2 Done 2026-08-09, read the ledger entry rather than redoing it

1. **Item 0.4 (A3)** — the three `capi`-layer X11 tests exist. The seam this
   document named, `LinuxX11Context::from_render_display_for_test`, was `#[cfg(test)]`
   and therefore invisible to `capi`; `migo-platform/test-support` and
   `X11TestServers` replace it, and replace the old fake rather than joining it.
2. **Items 0.2 (A1) and 0.3 (A2)** — mutants taken. A2's is textbook: only the
   static contract sees the reordering. **A1's did not fail, and could not** — see
   §4's new entry; the fix was to make the state unrepresentable
   (`capi/src/retirement.rs`), not to tighten the probe.
3. **Task T.4's permission cluster** — already killed in the previous round; this
   document's description of it was stale. Re-measured: 5 survivors, every one a
   mutator writing the constant that is already there. Nothing to do.
4. **Task T.6** — a derived gate was reading Java bytecode it did not compile, so it
   failed on a cold tree and passed on a stale one. Fixed at the gate.
5. **Task T.8, new** — `verify-change.sh` claimed OpenHarmony has no local build.
   It has one, it takes 13 seconds, and `crates/capi/src/platform/ohos.rs` was
   compiled by *nothing* on this machine. There is now an `ohos compile` lane.
   `windows` is still genuinely absent and still `NOT PROVEN`.

### 3.3 Next, in value order

1. **The remaining phase-0 epics**: 0.7/A7 (Android capability enforcement, 30
   protected and 8 cleanup operations), 0.8/A8, 0.9/A9, 0.10/A10, 0.11/A11, 0.13
   HarmonyOS, 0.14/A13. Most have their own detailed plan named in the ledger entry.
2. **Phase 1 hermetic builds** — `part-phase-1.md`, 18 open items.

**Before starting any of them, check the recorded obstacle against the object it
names.** The count of wrong ones reached **fifteen** on 2026-08-09, and the
fifteenth was in this document: the seam item 0.4 named was real but sat at a layer
the crate needing it cannot see. The reflex that dissolves most of them is asking
*"which layer can see this property?"* rather than *"how do I reach this code?"*.

## 4. Traps that cost real time here

- **The ledger's new part files are invisible to git, and committing without them
  would publish an index pointing at nothing.** `.gitignore` line 116 is `docs/`.
  The ledger and spec predate that rule and are tracked, so `git add -u <path>`
  updates them -- but the four `2026-08-03-four-platform-delivery/part-*.md` files
  created by the split are new, and `git add` refuses them as ignored. They need
  `git add -f`:
  ```bash
  git add -f docs/superpowers/plans/2026-08-03-four-platform-delivery/*.md
  git add -u docs/superpowers/plans/2026-08-03-four-platform-delivery.md
  ```
  Re-including them through `.gitignore` is not possible without rewriting that
  rule: git does not descend into an excluded directory, so a `!docs/superpowers/`
  negation under `docs/` has no effect.

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
- **A gate can be absent rather than weak, and a crate-name comparison cannot see
  that.** The contract compared the local and CI suite lists one way only,
  `local ⊆ CI`, which is the harmless direction. `CI ⊆ local` is the one that makes
  the *local verdict false*, and `migo-capi-abi` sat in it for months. Scope is not
  visible in a name either: `test -p migo-capi-abi --lib` names the crate and runs
  zero of its 60 tests. Ask `cargo metadata` what targets exist (task T.7).

- **A mutant killed by the compiler yields no evidence.** Reintroducing the Qt
  held-button cache left `dom_button_held` unused, and the host kit builds with
  `-Werror=unused-function`, so the run failed to compile instead of failing a named
  test. The mutant has to be the *whole* original shape, dead helper removed.

- **Pick the mutant a point sample cannot survive.** A one-pixel shift of
  `clearRect` walked past the golden test, because it sampled one interior pixel and
  one far corner — both preserved by the shift. Before trusting any pixel or field
  assertion, ask what it admits.

- **A test-only restatement of a production rule is a warning sign.**
  `active_methods` selects the Android JNI surface with a `#[cfg(feature)]` chain and
  `methods_for` states the same rule declaratively under `#[cfg(test)]`. Every test
  asserted over the one that never ships, so deleting a line of the production chain
  was a survivor. The missing test is always the one that equates them.

- **Guards cover the side they were designed for.** Twice in one session a new
  guard passed its own mutant while the defect it existed for survived. After
  writing one, ask what it does *not* cover, and write the mutant that exploits
  that.

- **A sampling probe cannot observe "held nothing during a blocking call."**
  `engine_destroy_holds_no_engine_lock_while_joining` read `try_lock` once from the
  thread being joined and passed **50/50** with a lock deliberately held across the
  join: the sample had no ordering against the join and ran before the destroying
  thread was scheduled. Spinning until the lock looked free failed too, because the
  real code releases it and the mutant re-acquires it, so the awaited state does
  occur. The property is structural; the fix was `capi/src/retirement.rs`, where the
  `Mutex` is private to its own module and the defect no longer compiles. Whenever a
  test's subject is "what was *not* happening during a blocking call", expect this.

- **Prove the mutant is in the binary before believing a survivor.** `touch` and a
  `Compiling <package>` line are necessary, not sufficient. The cheap positive
  control is a second, obviously-fatal edit a line or two away: if *that* fails the
  test, the function you mutated is what ran. It cost one 30-second build and it is
  what made "this shipped guard cannot fail" safe to write down.

- **A gate can read a build artifact it does not produce.** Then it fails on a cold
  tree and *passes on a stale one*, which is the direction that matters. Found in
  `test-android-host-api-contract.sh`; details and the comparison run are in T.6.

- **`pitest` cannot run `--offline`.** Its JARs live in the `pitestRuntime`
  configuration and are not in the offline cache, so
  `./gradlew --offline :library:pitestFullDebug` dies in dependency resolution. Run
  it without `--offline`; the focused form is
  `-PpitestClasses=<glob> -PpitestTests=<TestClass>` and takes about twenty seconds.
  Read `library/build/reports/pitest/mutations.xml`, not the HTML.

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
