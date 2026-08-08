# Four-Platform Delivery — Handoff, 2026-08-08

**Authority:** `docs/superpowers/specs/2026-08-03-four-platform-delivery-design.md`.
**Ledger:** `docs/superpowers/plans/2026-08-03-four-platform-delivery.md` — the
status convention, and every closed item's evidence, lives there. Read its
"Status Convention" section before marking anything.

**Branch:** `master`. The `perf/ble-notification-path` work merged as `0a53199`
(#30), so nothing here depends on an unpushed branch any more.

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

### 3.1 A12 (ledger task 0.12) — done, pending only the two reviews

**Superseded by the ledger.** All three clauses are settled and the evidence,
including the `verify-change.sh` verdict block and every mutant, is in ledger item
0.12. In short:

- **Pixel ratios:** already satisfied; the clause was stale and is corrected.
- **Game identity:** the "Windows diverges" premise was false — one shared rule,
  zero `#[cfg]`. The live defect was that the rule was case-*preserving*, so
  `PuzzleQuest` and `puzzlequest` became one directory on NTFS/APFS. Fixed by
  narrowing the id space to lower case (making the folding pair unrepresentable
  rather than resolved per platform) and rejecting reserved device names on every
  platform. Both language gates now read one vector table,
  `engine/crates/shared/src/vfs/game-id-vectors.txt`, so widening one side alone
  fails that side.
- **Missing ad handler:** every one of the six ad entry points now settles what
  content waits for, and the *interface defaults* settle identically — that second
  half was the larger hole, because a registered handler that does not sell
  rewarded video stalled content exactly as a missing handler did. The settlement
  method is abstract on the command enum, so a seventh ad command cannot compile
  until somebody decides what it settles as.

**Two things this turned up that are now the top of the list.**

### 3.2 Done this session: the contract lane, the Slim profile, pitest, the split

Recorded in the ledger with evidence; listed here only so the next session does not
redo them. Ledger tasks T.4, T.5, T.6 and item 0.15.

- **The local verifier ran none of the ~24 source-structure contract gates** -- they
  lived only in `.github/workflows/pr-ci.yml`. Found by A12's own mutant: reverting
  one ad entry point to a bare handler lookup passes every unit test in both
  languages and is caught by one contract script, so the local gate called a change
  "verified" that CI rejects. The lane is now derived from the workflow (so it
  cannot drift from CI), runs on every invocation, and is pinned by six new checks
  in `scripts/test-local-verification-contract.sh`. **Its own first version
  under-ran silently** -- a gate that runs `cargo` drained the here-string the loop
  was reading, and three gates plus a verdict line vanished from a run that still
  reported success. Read T.6 before touching it.
- **Nothing had ever run a Slim host suite**, and the first one reported 36
  failures. Six were a real defect: the window-resize ingress lived in the
  `api-connectivity` extension, so on a Slim build no canvas ever followed its
  surface. Both Slim suites are host steps now.
- **pitest is wired** (`:library:pitestFullDebug`), and its 57 production survivors
  are listed in T.4. The published Gradle plugin cannot be used on an Android
  library module; read T.4 before reaching for it.
- **The ledger is split** per phase with a stable index, so several agents can work
  without colliding on one 5,900-line file.

### 3.3 Make the host suites selective (speed, ~1 session)

`scripts/verify-change.sh` runs **all** host cargo suites on every invocation, and
there are now sixteen of them, so a Java-only change pays for every Rust suite plus
both Slim profiles. `scripts/lib/verification_targets.py` already maps changed files
to crates (`_CRATE_PATH`), so the missing piece is a reverse-dependency closure from
`cargo metadata`: run the changed crate's suite plus every crate that depends on it,
dev-dependencies included.

**Get this right or not at all** -- under-running is a silent gap, which is strictly
worse than slow, and the contract lane above already demonstrated how quietly it
happens. Fail closed: anything outside `engine/crates/**` (`Cargo.lock`,
`.cargo/config.toml`, a workspace manifest, a script), or a `cargo metadata` that
does not run, must fall back to every suite and say why.
`scripts/test-local-verification-contract.sh` must grow an assertion that a change
in a leaf crate still runs its dependents. **Leave the contract lane
unconditional** -- each gate is seconds, and keying them to changed files means a
file list per gate, which is a list to forget an entry from.

### 3.4 Kill the pitest survivors, starting with the permission ones

T.4 lists 57. The ones that matter on this project's own standard that a guard which
cannot fail is decoration: `PermissionOperationGate` and `PermissionRevocation`
survive having their return values inverted, `BluetoothManager.hasConnectPermission`
survives both negation and a forced `true`, and `NativeMethods.updatePermission`
survives negation of its whole argument guard. `TouchEventHandler`'s 19 are the
largest cluster.

### 3.5 The rest of A6, and then the open epics

Task 0.15 ran the two crate suites under both profiles; the lifecycle,
reattachment, input-saturation, ABI and header contract suites it also names have
not been run under Slim, and the last two need the C package rather than a host
suite. After that the open work is the epics in phase 0 (A1, A2, A5, A7 through
A11, HarmonyOS) and phase 1's hermetic builds, several of which need the device,
the emulator or the Windows toolchain named in Section 0 and cannot be closed on
this machine.

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
