> Part of the [Four-Platform Delivery Ledger](../2026-08-03-four-platform-delivery.md).

## Phase 1 — Hermetic Four-Platform Builds

> **Spot-audit 2026-08-09, and the result reverses the prior this ledger has been
> building.** Seven phase-0 items audited this month turned out already
> implemented, so it was reasonable to expect the same here. It does not hold:
> six phase-1 items were checked against the objects they name and **all six are
> genuinely open**, with nothing started.
>
> | Item | Checked | Found |
> |---|---|---|
> | 1.2 one release-version source under `release/` | directory listing | `release/` does not exist |
> | 1.4 remove every `--allow-multiple-definition` | tree-wide grep | still in `engine/.cargo/config.toml` ×5, `platforms/openharmony/.../CMakeLists.txt`, and `scripts/build-android-so.sh`, whose comment calls it required on every Android target |
> | 1.6 release must reject `--skip-rust` | grep | `scripts/build-aar.sh` still documents and accepts it |
> | 1.9 HarmonyOS V8 component manifest | directory listing | neither `*-linux-ohos/` has `component-manifest.json`; `x86_64-linux-gnu/` does, which is the shape to copy |
> | 1.10 two-sysroot HarmonyOS floor | ran it, then ran it properly | **satisfied on both architectures the same day** — the newer SDK was already installed at `~/ohos-sdk-6.1` (6.1.0.31, API 23). See the entry below |
> | 1.12 byte-equal rebuild for every shipping archive | `ls scripts/` | no reproducibility or determinism gate exists |
>
> **Budget these as implementation, not as audits.** **1.10 turned out to need
> nothing at all** — see its entry. **1.4 is the one to think about
> before touching**: `--allow-multiple-definition` is masking duplicate symbols
> that Skia and V8 each contribute, so removing it is resolving those symbols per
> platform, not deleting a flag.

- [x] 1.1 Build the Android V8 archives from source. **Both architectures now
  reproduce bit-identically.** `aarch64` produced
  `engine/third_party/rusty_v8/aarch64/librusty_v8.a` at 121,784,912 bytes, sha256
  `681aaa39367a9aa35ab7e584ddd4b36273acbc0ccb4177648c43b9b55b7eb273`, matching the
  recorded `hashes.archive`. `x86_64` followed in 85m31s at 124,638,610 bytes,
  sha256 `ce14223a4d938011b888a3ea32ffd795ae1904d86e71989321e0cec2cfee5413`, also
  matching its recorded `hashes.archive`, with `src_binding.rs` matching
  `hashes.rust_binding` at `47369bfb3ee9a7e6e0f451ae20242595d5a7f0b3afcc0b9447e4f99ff174686a`.
  Two independent architectures reproducing the committed hashes from source is the
  evidence that the prebuilt archives are what they claim to be, and that the build
  is deterministic rather than incidentally matching once.
  The three prerequisites are resolved: the source tree's submodules are
  initialised at V8 14.5.201.2, the git ownership exception turned out to be
  irrelevant because the build script runs no git command in that tree, and `gn`
  is built from source at revision 2502 and installed at the prefetch path
  `third_party/v8_correct_gn/gn` that the script probes before the system `PATH`.
  Two defects had to be fixed first, both committed as `2287460`: the
  custom-libcxx patch declared eight new-side lines for a hunk carrying seventeen,
  so GNU patch rejected the file as malformed and no Android V8 archive could ever
  be produced; and every patch's applied-check sentinel came from its first hunk,
  so a patch dying part-way would leave the sentinel behind and make the next run
  skip it, building V8 with the snapshot-toolchain libc++ logic silently missing.
  Toolchain provenance to record with the artifact: gn revision 2502
  (`17b0057970fa`), built with host gcc and carrying one local one-line change
  that adds `-Wno-comment` to gn's existing gcc-only suppression list, because
  gn's `header_checker.h` ends an ASCII-art comment line with a backslash that
  only gcc reports and gn compiles with `-Werror`. Android NDK 23.2.8568313,
  API 26.

  Neither build could seal a fresh manifest: both ended with
  `V8 component manifest: rusty_v8 revision failed: fatal: detected dubious
  ownership`, so each archive is verified against the pre-existing manifest rather
  than a newly written one. That is task 1.1f, now confirmed on a live run rather
  than inferred from reading the writer.
- [x] 1.1a Pin the Android NDK and resolve it by identity. The repository declared
  no `ndkVersion` and no NDK pin anywhere, so the V8 archive and the AAR could be
  produced with different NDKs without anything noticing. Inherited specification
  Section 8.3 requires the NDK to be pinned.

  Worse than unpinned, the path was guessed. Seven scripts independently defaulted
  `ANDROID_NDK_HOME` to `$HOME/Android/Ndk` — **a path that exists on none of the
  machines this has been built on**; the NDK actually in use is at
  `$HOME/Android/Sdk/ndk/23.2.8568313`. Every successful build therefore depended on
  the variable already being set correctly in the environment, and a fresh checkout
  on a new machine would have failed on a path nobody ever had.

  The lock now pins `ndk.version` to `23.2.8568313` — the NDK that produced both
  verified archives, so the pin is what is proven rather than what is newest — and
  `scripts/lib/android-ndk.sh` *finds* it, searching `ANDROID_NDK_HOME`,
  `ANDROID_NDK_ROOT`, `$ANDROID_HOME/ndk/<version>`, `$ANDROID_SDK_ROOT/ndk/<version>`
  and `$HOME/Android/Sdk/ndk/<version>`. Selection is by the NDK's own
  `Pkg.Revision`, not by directory name: a directory called `ndk/23.2.8568313` is
  just a name, and `Pkg.Revision` is the same fact the component manifest already
  stamps into the artifact. An explicit `ANDROID_NDK_HOME` is honoured but checked
  like any other candidate, so an override cannot substitute a different toolchain
  silently.

  `scripts/test-android-ndk-pin-contract.sh` covers it over a synthetic SDK layout,
  so the fixtures do not depend on which NDKs the machine has: a matching NDK is
  selected; a directory carrying the pinned *name* while reporting a different
  revision is refused (the case a path-only check cannot see); a directory with no
  `source.properties` is refused; resolution fails when nothing matches; an override
  pointing at the wrong NDK does not win; an override pointing at the pinned NDK is
  honoured. It also proves this machine resolves the pin with `ANDROID_NDK_HOME`
  unset — the case that was silently broken — and that `build-v8-android.sh` no
  longer defaults the variable at all.

  **Done under task 1.1k:** the eight other scripts that consume the NDK now route
  through the resolver too, so the AAR and the V8 archive cannot be built by
  different NDKs.
- [x] 1.1k Route every Android script through the pinned NDK resolver. All eight
  remaining scripts now call `android_ndk_read_pin` + `android_ndk_resolve`:
  `build-android-c-host.sh`, `build-android-sdk.sh`, `gen-snapshot.sh`,
  `test-android-sdk-contract.sh`, `test-android-symbol-floor.sh`,
  `test-capi-platform-contract.sh`, plus `build-android-so.sh` and `build-aar.sh`,
  which had required `ANDROID_NDK_HOME` without checking which NDK it named. So an
  AAR can no longer be built with a different NDK than the V8 archive it links.
  `build-android-so.sh` keeps exporting `ANDROID_NDK` for skia-bindings, which reads
  that name rather than `ANDROID_NDK_HOME`.

  The contract check is now an **enumeration, not a list**: every script mentioning
  `ANDROID_NDK_HOME` must resolve it, so a script added later cannot reintroduce the
  guess unnoticed. That change immediately found four scripts a list-based sweep had
  missed — `build-linux-sdk.sh`, `dev-run-c-host.sh`, `dev-run-player.sh` and
  `dev-test-host.sh` — because they mention the variable without using the
  `:-$HOME/...` default pattern. Inspection showed all four only `unset` it: host
  builds deliberately refusing an ambient Android toolchain, which is the opposite
  of guessing a path. The predicate "mentions it, therefore must resolve it" was
  too strong and would have failed correct code, so scripts whose only mention is an
  `unset` are classified as refusing rather than failed. 13 scripts, all correctly
  classified.

  Removing the guessed default from `test-android-symbol-floor.sh` made its
  in-function re-derivation of `REPO_ROOT` dead, so it is gone.

  One correction from review: `test-android-sdk-contract.sh` deliberately treats a
  missing NDK as a **skip**, with `--strict` being what turns a skip into a failure.
  Making resolution fatal there took the manifest, export and snapshot checks — none
  of which need an NDK — down with it. Resolution failure now leaves `NDK_BIN` empty
  and the existing skip accounting handles it. Verified by running the script with
  every SDK root pointed at a nonexistent path: it prints the resolver's diagnostic
  and proceeds to its own checks instead of aborting.

  Verified: `bash -n` on all eight; the NDK pin contract passes at 22 checks; every
  variable each new `source` line depends on is defined above it. Three scripts that
  do not need a device were run and all got **past** NDK resolution, failing instead
  for their own reasons — no staged `libmigo.so`, no staged package, and a cargo
  check this run declined to complete. The five that drive cargo, gradle or ninja
  were not run end to end.
- [x] 1.1b Make the V8 build lock the single declaration of the applied patch set.
  The lock pinned three `required_patches` while `scripts/build-v8-android.sh`
  applied four, and `scripts/write-v8-component-manifest.py` carried its **own**
  id-to-filename table that also listed three. So the prebuilt-binding diff was
  applied by every build and recorded by neither the lock nor the sealed manifest:
  the manifest asserted a patch set the build had not used.

  Rather than add the fourth entry in three places and assert across them, the
  three declarations were collapsed into one. `required_patches` entries now carry
  `id`, `file` and `notes`; the writer's `PATCH_FILES` table is deleted and it
  resolves straight from the lock; and the build script's literal
  `V8_DECLARED_PATCHES` array is gone, read from the lock instead. Divergence is now
  impossible by construction rather than caught by a comparison — which is the same
  reasoning that removed the sentinels in task 1.1e. Verified: both consumers
  independently resolve the same four files, and the prebuilt-binding diff enters
  the manifest's patch provenance for the first time at sha256 `db9f843bb8a3…`.

  The per-patch rationale moved into the lock's `notes`, where it sits next to the
  thing it explains instead of in a build-script comment that no consumer reads.

  Two findings came out of doing it:

  The contract check for "every declared patch resolves to exactly one file"
  **silently narrowed from 8 patches to 4** the moment Android's literals moved into
  the lock, because it only grepped build scripts. The zero-matches guard added
  earlier did not catch it, since 4 is not 0. It now draws from both sources —
  script literals and lock `required_patches[].file` — and fails if *either* source
  contributes nothing, so a whole source going silent is caught rather than absorbed.

  Adding a file-mode comparison to the replay proof (prompted by
  `write-linux-v8-component-manifest.py`, which already compared modes and so set the
  standard) immediately failed on the real tree for
  `build/rust/gni_impl/run_bindgen.py`. The defect was in the replay, not the tree:
  `git show` writes through the umask and drops the recorded mode, so the pristine
  copy always came out non-executable and the comparison was measuring the
  materialisation rather than what the patches produce. The replay now reads the
  mode from `git ls-tree` and applies it. `run_bindgen.py` is `100755` in HEAD, and a
  synthetic fixture proves a flipped executable bit on a patched file is refused.

  Two more came out of review, both of them fresh damage from this change:

  The artifact-manifest contract fixture staged a **hardcoded three** patch files
  while its lock fixture now declared four, so the writer reported the fourth
  missing and, under `set -e`, the whole contract died before either manifest was
  generated. Same restatement problem one level down; the fixture now stages
  whatever its own lock declares. Verified by running
  `scripts/test-artifact-manifest-contract.sh` to completion: `ok`, rc 0.

  Reading the declaration with `mapfile -t X < <(python3 …)` was **fail-open**. A
  process substitution's exit status is not what `||` observes, so a parser that
  printed a valid prefix and then rejected a malformed entry left a truncated
  declaration behind and passed the non-empty guard — the build would have applied
  fewer patches than the lock requires and only discovered it at manifest time,
  after producing artifacts. Proved concretely: the old form keeps two entries from
  a parser that exits non-zero after printing two lines, and `||` never fires. It is
  now a command substitution, whose status does propagate, plus a non-empty check.
  Verified against a lock whose third entry has no `file`: the build refuses instead
  of continuing with two patches.

  **Consequence to settle under 1.1f:** the three committed
  `component-manifest.json` files still record the old three-patch set, so they are
  now stale rather than wrong — nothing compares a manifest's patches against the
  lock at validation time, so they still validate. Regenerating them is blocked on
  1.1f, and doing so will change each `component_id`.
- [ ] 1.1l Wire or delete `required_patches` in the Windows V8 lock.
  `contracts/artifact-manifest/windows-v8.lock.json` declares three
  `required_patches` and **nothing reads them**:
  `scripts/write-windows-v8-component-manifest.py` never references the field, and
  `scripts/build-v8-windows.sh` names its patches as its own literals. So the
  Windows lock states a requirement it does not enforce, which is the same
  declared-but-unenforced shape task 1.1b just removed from Android. Give it the
  `id`/`file` shape, have the Windows build and writer read it, or delete the field
  rather than leave it decorative. The Windows build is not runnable on this host, so
  this needs a machine that can run it.
- [x] 1.1c Pin and assert `gn`. The script resolved `gn` from `V8_GN_PATH`, then a
  prefetched path, then the system `PATH`, and merely **logged** the version it
  found — so a system `gn` of any revision was accepted and failed later somewhere
  confusing. gn generates the entire build graph, so it is an input to every byte
  of the archive.

  The lock now carries a `gn` block pinning version `2502` and the full revision
  `17b0057970fa2b07a20cbb4289ab78cf93565f35`, and `scripts/build-v8-android.sh`
  refuses to build unless the resolved `gn` matches, before it invokes cargo.
  `scripts/lib/gn-pin.sh` is the single reader of that block so the builder and
  any future consumer cannot disagree about what the lock says.

  Both halves of the reported identity are checked, but **neither is sufficient**,
  and the first draft of this work got the reason wrong. `gn` reports
  `<version> (<short-revision>)` where the version is a commit position — and the
  revision comes from the same `git describe HEAD`, with no dirty marker. So
  checking the revision detects a locally modified gn no better than checking the
  version does: a gn built from the pinned commit *without* the required patch
  reports exactly the same string as the intended one. The declared patch
  provenance was therefore unenforced by the version check alone.

  What enforces it is a receipt. `scripts/build-gn.sh` writes
  `gn-receipt.json` beside the installed binary, recording the revision, the
  sha256 of every patch it applied, and the sha256 of the binary itself;
  `gn_pin_assert_binary` requires the receipt to match the lock and the binary to
  still hash to what the receipt says. The receipt for the gn in use was written
  only after confirming the installed binary is byte-identical to the `out/gn` of a
  checkout proved to be HEAD plus exactly the declared patch — so it records a
  verified fact rather than an assumption.

  That local modification is no longer unrecorded. It is committed as
  `engine/third_party/gn-patches/0001-suppress-gcc-multiline-comment-warning.patch`
  — one line adding `-Wno-comment` to gn's existing gcc-only suppression list,
  needed because `header_checker.h` ends an ASCII-art comment line with a backslash
  that only gcc reports and gn compiles with `-Werror`. `scripts/build-gn.sh`
  reproduces the pinned gn from a checkout at the pinned revision, applying that
  patch through the same reverse-apply probe the V8 patches use, and installs it at
  the prefetch path. So `gn` is a reproducible input rather than a binary that
  happens to be on this machine.

  Applying the declared patches proves each of them landed; it does not prove
  nothing else did. `v8_assert_tree_is_exactly_patched` closes that by replay:
  materialise every modified path at HEAD, apply the declared patches to that, and
  require the result to equal the worktree byte for byte. An edit smuggled into a
  patched file, an edit to an unrelated file, and a stray untracked file are each
  refused. This is the check task 1.1d needs for the rusty_v8 tree; it does not yet
  descend into submodules, which that task must add.

  Two further environment defects were fixed in `scripts/build-gn.sh`. It defaulted
  its install prefix to a hardcoded absolute path, so a clone anywhere else would
  install where the V8 build does not look; both the source and prefix defaults now
  derive from the repository's own location. And it left the host compiler to gn's
  `clang++` default, which resolves to the Android NDK's clang 12 here because the
  NDK precedes the host compiler on `PATH` — and clang 12 rejects the C++ standard
  gn compiles itself with, so the documented recovery command would have failed.
  The script now reads the required standard out of gn's own `build/gen.py` and
  probes the candidate compiler against it, rather than matching version numbers.

  Aligning the defaults exposed a separate latent defect: `build-v8-android.sh`
  defaulted `RUSTY_V8_SRC` to `/home/wkspace/rusty_v8_src`, a path that does not
  exist on this machine, while `build-v8-linux.sh` and `build-v8-ohos.sh` both
  already derive it from `$PROJECT_ROOT/..`. The Android script now matches them, so
  `build-gn.sh` installs where the Android build actually looks.

  The receipt only means something if it describes bytes the builder compiled.
  `build-gn.sh` therefore removes gn's output directory before generating: `out*` is
  gitignored, so the replay proof cannot see anything living there, and an existing
  `out/gn` with a recent mtime — a hand-built or copied binary — would leave ninja
  with nothing to do, after which the script would install that binary and write it
  a perfectly matching receipt.

  The revision comparison also needed a length floor. `"$short"*` as a prefix test
  accepts an empty or one-character abbreviation, so `2502 ()` and `2502 (1)` both
  passed as valid prefixes of the pinned sha; the abbreviation must now be 7 to 40
  hexadecimal characters, and the three malformed forms are fixtures.

  A `minimum_version` field was written and then **removed**: V8 14.5 does need gn
  2315 or newer for `path_exists()`, but an exact version pin already refuses
  everything below it, so a minimum could never be the reason a gn was rejected.
  The 2315 requirement lives in the lock's `notes`, where a rationale belongs.

  `scripts/test-v8-gn-pin-contract.sh` covers it in 29 checks: the lock declares a
  full 40-char revision and at least one required patch, every declared patch is
  committed, the real prefetched gn satisfies both the version pin and its receipt,
  and eleven version strings are checked including an older version, a newer
  version, a matching version with a foreign revision, a revision that is a
  near-miss rather than a prefix, an empty revision, a one-character revision, a
  non-hexadecimal revision, a string with no revision, and a non-numeric version.
  Five receipt fixtures cover a gn with no receipt, a valid receipt, a receipt
  claiming no patches, a binary altered after its receipt was written, and a receipt
  naming a different revision. Five replay fixtures cover a pristine tree, an
  exactly-patched tree, an extra edit inside a patched file, an edit to an untouched
  file, and a stray untracked file. It also asserts the build script performs the
  assertion **before** the cargo invocation, since an assertion that runs after the
  build has consumed gn cannot keep an unpinned gn out of the artifact. A first
  draft of that last check grepped for an unchecked `gn --version` log line and
  fired on the *correct* implementation, which captures the version precisely in
  order to assert it; a check that fails on correct code is worse than no check, so
  it was replaced by the ordering property.

  **Not verified:** the end-to-end Android build has not been re-run since the gn
  assertion was added, because that is an 85-minute build, and `scripts/build-gn.sh`
  has not been run end to end since it was rewritten to build from a clean output
  directory. The assertion path itself is exercised against the real lock, the real
  gn and its real receipt by the contract test.
- [ ] 1.1j Record `gn` in the component manifests. Deliberately **not** bundled with
  1.1c after the cost of doing it honestly became clear. `toolchain.gn` was written,
  wired through `--gn`, and then reverted rather than shipped half-migrated, because
  `migo-v8-component-manifest/v1` is an explicitly cross-platform schema — Android,
  Windows and Linux all seal through `seal-v8-component` and share
  `ToolchainIdentity`. Adding `gn` as an `Option` is the wrong shape: the schema's
  own doc comment records the rule that widening a shared type until every field is
  optional "is precisely how a missing floor becomes unnoticeable". So the change is
  a new V8-specific toolchain type with a **required** `gn`, which forces all three
  platforms at once.

  Two of the three can be pinned honestly today. `scripts/build-v8-linux.sh:173`
  resolves gn through the identical `V8_GN_PATH` → prefetched → `PATH` chain and has
  the same unpinned-gn hole; it uses the same prefetched binary, so it can carry the
  same pin. Windows gets gn from rusty_v8's own `ninja_gn_binaries.py` download
  inside the build, and there is no verified identity for it here — pinning it would
  mean inventing provenance. Establish the Windows gn identity first, then make the
  field required across all three and regenerate the three committed
  `component-manifest.json` files, which the schema change invalidates. Regeneration
  is blocked on 1.1f.
- [x] 1.1d Keep the rusty_v8 working tree free of unversioned drift, and prove it.
  The gate is now `v8_assert_tree_is_exactly_patched`, wired into
  `apply_patches` so every Android V8 build refuses a tree carrying a change no
  committed patch explains.

  The check is a replay rather than an allowlist. The previous mechanism — the
  manifest writer's `check_source_changes` — compares modified paths against a
  hardcoded set (`build.rs`, `build/rust/gni_impl/run_bindgen.py`,
  `build/config/c++/c++.gni`), which is the sentinel defect class again: a
  hand-maintained restatement of which files the patches touch, drifting the moment
  a patch grows a file. And a path allowlist cannot see an edit *inside* an allowed
  file. The replay materialises every modified path at the HEAD of whichever
  checkout owns it, applies the declared patches to that, and requires the result to
  equal the worktree byte for byte.

  Submodule descent is the part that makes it usable here: two of the four Android
  patches (0001, 0003) land inside the `build` submodule, which surfaces in the
  parent as a single opaque gitlink entry. Without descending, every submodule
  change reads as one undeclared modification.

  The patch list is now declared once, as `V8_DECLARED_PATCHES`, and read by both
  the applier and the gate — a second literal list for the gate would have been the
  same drift risk this task exists to remove.

  Result on the live tree: `/data/work/opensource/rusty_v8_src` is **proved** to be
  HEAD plus exactly the four declared patches. Root carries only `build.rs`
  (0002 + the prebuilt-binding diff); the `build` submodule carries only
  `config/c++/c++.gni` (0003) and `rust/gni_impl/run_bindgen.py` (0001); the `v8`
  submodule is clean; no stray stamp file remains. The two untracked files under
  `third_party/v8_correct_gn/` are the pinned gn and its receipt, declared in
  `V8_ACCOUNTED_PATHS` because their provenance is the receipt checked by 1.1c,
  named as exact paths so a new file appearing beside them fails the gate rather
  than inheriting the exemption.

  Proved load-bearing against the real tree by two mutations: dropping the
  accounted-path list makes the untracked gn a refusal, and disabling submodule
  descent makes ` M build` an opaque undeclared change. So the pass depends on both
  mechanisms rather than being vacuous. Eleven synthetic fixtures over a real
  git-submodule fixture cover a pristine tree, an exactly-patched tree, an extra
  edit inside a patched submodule file, an edit to an untouched submodule file, an
  extra edit inside a patched top-level file, a stray untracked file, that same file
  once exempted, a near-miss exemption that must not cover it, an exemption that
  must not persist once it stops being passed, and a submodule moved off its pinned
  commit. The contract test also runs the gate against the real checkout on every
  invocation, reading `V8_DECLARED_PATCHES` and `V8_ACCOUNTED_ARGS` out of the build
  script so the test cannot drift from what the build declares.

  Three corrections came out of review, all of them holes in the first draft:

  Descending into a submodule was **unsound without checking the gitlink**. A
  submodule checked out at some other commit where the declared patches still apply
  was reported clean, because the replay took that foreign HEAD as its pristine
  baseline — and the manifest writer never checks the `build` submodule revision, so
  an artifact built from unpinned sources could have been sealed. Descent now
  requires the submodule's HEAD to equal the commit its parent records, and a moved
  gitlink is reported as the undeclared change it is. The live tree passes, so
  rusty_v8's `build` submodule is confirmed to be at its pinned commit.

  Exemptions were read from an **ambient variable**, so an exported
  `V8_ACCOUNTED_PATHS` in a release environment could have granted one — and
  `build-gn.sh` never initialised it, so it was inheritable there. They are now
  `--accounted <path>` arguments: a caller that needs an exemption has to say so at
  the call site.

  Worst of the three: the mutation sensitivity checks had been passing **vacuously**
  ever since the library grew a `source host-requirements.sh` line. That path is
  relative to the library, so a mutant copied into a bare `mktemp` file resolved it
  to `$TMPDIR` and failed to load at all — every fixture then failed for an
  unrelated reason, which reads as perfect sensitivity. This is precisely the shape
  of false confidence the whole contract exists to prevent, self-inflicted. Mutants
  are now built in a directory beside a copy of the dependency, and `make_mutant`
  verifies each one both differs from the original *and* actually loads and defines
  the function under test before any conclusion is drawn from it.

  **This is why nothing is committed into the vendored trees.** Committing the
  local modifications in `rusty_v8_src` or `gn` would move each HEAD off the
  revision its lock pins, which is exactly what the manifest writer's
  `rusty_v8_revision` comparison and `build-gn.sh`'s revision assertion are there to
  detect — the provenance chain would break, and the change would become
  unreproducible from a clean clone. The modifications are captured as committed
  patches in this repository instead, and the replay proof above is what establishes
  that the capture is complete rather than assumed.

  Still open here: `maybe_install_sysroot` in rusty_v8's `build.rs` probes
  `build/linux/debian_sid_{arch}-sysroot`, a name Chromium retired in favour of
  `debian_bullseye_*`, so the guard never matches and `install-sysroot.py` runs on
  every build. It is stamp-guarded and does not re-download, but it makes the
  Android build contact the network unconditionally, which an offline or sealed
  release build cannot tolerate. Fixing the probe makes the sysroot a declared,
  cacheable input instead of a per-build fetch; it needs a new committed patch
  against `build.rs`, which the gate will then require.
- [x] 1.1e Decide patch applied-ness from the patch, not from a sentinel string.
  `apply_patches` encoded each spec as `target|sentinel|glob` and extracted the
  sentinel with `${rest%%|*}`. Patch 0002's sentinel is
  `target_os == "linux" || target_os == "android"`, which itself contains `|`, so
  it was truncated to `target_os == "linux" ` — a string that appears twice in the
  **unpatched** file. The check therefore reported "already in effect" forever and
  **patch 0002 had never once been applied**. This also invalidated an earlier
  conclusion that 0002 had been absorbed upstream.

  Widening the audit to the other platforms found the same class twice more.
  `build-v8-windows.sh` carried the identical `target|sentinel|glob` array, and its
  entry for `0007-windows-register-host-callbacks-from-rust.patch` checked only
  `src/V8.rs` — the **last** of the five files that patch touches. `patch` does not
  stop at the first failing file, so a run in which `src/cppgc.rs` failed still
  patched `src/V8.rs`; the next run then read the sentinel, reported "already in
  effect", and would have built with four of five files unpatched.
  `build-v8-ohos.sh` gated 0008 on `[[ ! -f "$TOOLCHAIN_GN" ]]`, which cannot see a
  file whose content has diverged from the patch.

  Rather than repair the encoding, the sentinel was removed as a mechanism: a
  sentinel restates what a patch does, so it can always drift from the patch. The
  three scripts now share `scripts/lib/v8-patch-apply.sh`, which asks `patch`
  whether the patch reverse-applies. That is derived from the patch itself and
  covers every file and hunk in it.

  The obvious spelling of that probe is itself a success-mask: with `--reverse`
  alone, GNU patch hits its `Unreversed patch detected!  Ignoring -R.` heuristic,
  decides the caller meant to *apply* the patch, applies it, and exits 0 — so an
  unapplied patch is indistinguishable from an applied one. Adding `--forward`
  turns that heuristic into `Skipping patch.` with a non-zero exit. `--fuzz=0`
  stops loose context matching from calling a hunk reversible against code it does
  not match. Verified on this tree: with `--reverse` alone, 0002 reported exit 0
  both when applied and when not.

  `scripts/test-v8-patch-application-contract.sh` covers this. Beyond the
  behavioural fixtures (unapplied, applied, second-run no-op, half-applied in both
  hunk orderings, drifted context, absent file) it re-runs every fixture against
  three deliberately broken copies of the library — one with `--forward` removed
  from the reverse probe, one with the forward dry-run preflight removed, one with
  `--fuzz=0` removed from the forward invocations only — and **requires all three to
  fail**. A guard that passes with and without its load-bearing flag is not a guard.
  The fuzz mutant is deliberately scoped to the forward invocations: stripping
  `--fuzz=0` from the reverse probe as well would make the drifted fixture fail
  because *application* became fuzzy, which says nothing about the probe, and a
  sensitivity check that cannot attribute the failure it observes is not evidence.
  The test also asserts no `build-v8-*.sh` invokes `patch` directly and that every
  patch literal any of them names resolves to exactly one file (8 of 8; the first
  draft of that extraction silently covered only 3, which a zero-matches assertion
  now prevents). 32 checks, and it passes under `env -i` as well as interactively.

  `patch --forward` is **not transactional**, which cost a second correction. Given
  a tree where an earlier hunk is unapplied but a later one is already applied —
  precisely the shape the old sentinel gate could leave behind for the five-file
  0007 — patch writes the earlier hunk, *then* reaches the applied one, skips it and
  exits non-zero. The build fails having left the tree more modified than it found
  it, and the next run starts from that new state. The first draft had a fixture for
  the opposite ordering only, which passes either way. `v8_require_patch` now runs a
  full forward `--dry-run` before the mutating invocation, so a tree the patch does
  not apply to completely is left exactly as found, and both orderings are fixtures.

  A post-apply "the result is reverse-applicable" assertion was written and then
  **deleted**: no fixture could make it fire, including the GNU patch
  trailing-newline asymmetry, because `patch --forward --fuzz=0` exiting 0 already
  implies every hunk landed. An assertion no fixture can trigger only manufactures
  confidence.

  Patch 0002 has now been applied to the tree for the first time
  (`build.rs:375`). Two consequences were checked before doing so rather than
  after. First, `use_sysroot=true` is now pushed twice, since the Android cross
  branch already pushes it unconditionally at `build.rs:406`; the successful
  aarch64 `args.gn` already contains `target_cpu = "arm64"` and
  `treat_warnings_as_errors = false` twice each, so GN demonstrably tolerates a
  redundant identical assignment. Second, `maybe_install_sysroot("arm64")` now
  runs, but `build/config/sysroot.gni` tests `is_android` **before** the
  `is_linux && use_sysroot` branch, so an Android target toolchain always takes
  `$android_toolchain_root/sysroot` from the NDK and never consults the Debian
  arm64 sysroot. The prediction is therefore that the aarch64 archive stays
  bit-identical at sha256 `681aaa39367a9aa35ab7e584ddd4b36273acbc0ccb4177648c43b9b55b7eb273`.
  **That prediction is not yet verified** — see task 1.1i.

  One further defect was found and not yet fixed: `maybe_install_sysroot` probes
  `build/linux/debian_sid_{arch}-sysroot`, but Chromium renamed those directories
  to `debian_bullseye_*`, so the guard never matches and `install-sysroot.py` is
  invoked on every single build. It is stamp-guarded and so does not re-download,
  but it makes the Android build reach the network unconditionally. Recorded under
  task 1.1d.
- [ ] 1.1i Verify the aarch64 archive is unchanged now that patch 0002 applies.
  Re-run `./scripts/build-v8-android.sh aarch64` and confirm sha256
  `681aaa39367a9aa35ab7e584ddd4b36273acbc0ccb4177648c43b9b55b7eb273`. The reasoning
  in task 1.1e says it must be, and a same-source reproducibility data point is
  worth having either way; if it differs, the reasoning about `sysroot.gni` or about
  duplicate GN args is wrong and the difference must be explained before the
  archive is trusted. The first run of this build will download the Debian arm64
  sysroot (~100MB) that the Android target never consults.
- [x] 1.1f The component manifest can be sealed again, and what it records is now
  reproducible. Both Android manifests were regenerated and re-verified:
  `aarch64` `component_id` `ee0fb437a33fcd9c…`, `x86_64` `1c6b7b20dde62eff…`, and
  `migo-artifact-manifest verify-v8-component` passes for both against their real
  archives. The archive and binding hashes are **unchanged**, so this describes the
  same bytes; the ids moved because the content of the description changed.

  The first blocker was git refusing the rusty_v8 tree at all — it is owned by
  another account on a shared group-writable workspace, so every `git rev-parse`
  and `git status` failed with `dubious ownership` and the writer could not seal
  anything. `scripts/lib/v8_source_proof.py` passes `-c safe.directory=<tree>` **per
  invocation** rather than writing it into the user's git config. This is git's
  documented mechanism for a repository you trust but do not own, and the trust it
  grants is already granted by the build, which executes that tree's `build.rs`.

  The second blocker was `check_source_changes`, a hardcoded list of allowed paths —
  the sentinel defect class once more: a restatement of which files the patches
  touch, blind to an edit *inside* an allowed file, and unable to account for the
  pinned gn and its receipt. It is replaced by the same replay proof the build-time
  gate uses, with the gn paths passed as `--accounted` arguments in the identical
  spelling `build-v8-android.sh` already uses for the shell side, so one array in
  that script feeds both.

  Regenerating is what exposed the manifest's own non-determinism, which no amount
  of reading would have shown:

  `toolchain.rustc` was whatever rustc happened to be on `PATH`. The committed
  manifest claimed `1.95.0`; regenerating produced `1.93.0`; and rusty_v8 pins
  **`1.89.0`** in its `rust-toolchain.toml`, so neither recorded version compiled
  anything in that tree. rustc is now resolved with the working directory inside the
  rusty_v8 checkout, so rustup reports the toolchain the tree pins.

  `toolchain.compiler` and `toolchain.linker` embedded the NDK's absolute path via
  clang's and lld's `InstalledDir`, so two machines building identical bytes produced
  different manifests and different `component_id`s. They are now normalised to
  `${ANDROID_NDK_HOME}` — `normalized_gn_args` already did exactly this substitution,
  and the toolchain banners had simply been missed. Verified: sealing twice in a row
  now produces byte-identical manifests.

  Three corrections were needed inside the proof itself:

  Only *changed* paths get a pristine blob materialised, but *all* declared patches
  were replayed, so a patch whose target was unmodified had nothing to apply to and
  the failure surfaced as patch's own `No file to patch` rather than naming the
  missing patch. Each declared patch is now required to be reverse-applicable
  against the real tree first.

  That check initially ran *before* the undeclared-change scan, which reported the
  less useful of two truths. An undeclared change names a concrete path, so it is
  reported first; the patch-applied check runs after it and before the replay.

  The artifact-manifest fixture could not satisfy a proof at all: its rusty_v8 stand-in
  is three files with no patches applied, which the old status-only check passed
  trivially. It now declares and applies its **own** one-line patch against
  `build.rs`, rather than the real V8 patches, whose contexts need their real target
  files at exact revisions. So the fixture exercises the proof mechanics instead of
  bypassing them, and its untracked-source case is now asserted to name both the
  violation and the offending path — a stronger assertion than the wording it
  replaced.
- [x] 1.1m Removed the duplicated source proof. `write-linux-v8-component-manifest.py`
  carried its own `changed_paths`, `patch_paths`, `head_blob`,
  `verify_exact_patch_result` and `git_revision` — the copy the shared module was
  extracted from, and the one that revealed byte equality alone is insufficient
  because the file mode must be compared too. Both writers now use
  `scripts/lib/v8_source_proof.py`; the Linux writer went from 412 lines to 242.

  Three of the Linux writer's separate checks collapse into the shared statement,
  because the proof descends into submodules: that the `build` submodule is
  pristine, that the `v8` submodule is clean, and that top-level changes fall within
  declared paths are all "every change must be one a declared patch accounts for".
  The result is a strictly better message. Against the real Android-patched tree the
  Linux declaration is refused with
  `undeclared change: M build/config/c++/c++.gni (no declared patch touches that
  path)` — the specific file — where it used to surface git's cryptic `" m build"`
  dirty-pointer status and needed a hand-written explanation to be intelligible.

  Two structural changes the merge forced, both of which make the shared module
  more correct than either original:

  Declared paths are now materialised for **every** declared path rather than only
  the changed ones. The Android version materialised the changed set, which left a
  patch whose target was unmodified with nothing to apply to; the Linux version
  materialised all declared paths but read them only from the root checkout, so a
  path inside a submodule could not be resolved at all. Now `submodule_paths` plus
  `_owner_of` route each declared path to the checkout that actually holds its blobs
  — necessary because a submodule's objects live in its own store, and
  `git ls-tree HEAD -- build/x` in the parent yields the gitlink for `build`, not
  the file. Verified across the 20 submodules of the real tree.

  Declaring **zero** patches is now a supported declaration meaning "this checkout
  must be pristine", which the Linux build makes when its prebuilt-binding diff is
  not in use. The Android version rejected an empty declaration outright.

  Two unit-test assertions pinned the old message wording and were updated to the
  new text, each with the intent stated: the drift message must name the file, and
  the submodule message must name the offending path rather than a status code.
  `scripts/ci/tests/test_write_linux_v8_component_manifest.py` passes at 5 tests,
  and the Android artifact-manifest contract still passes.

  Review then found a **P1 in the consolidation**: submodules were discovered from
  the parent's `git status`, and `submodule.<name>.ignore = all` (or `dirty`) makes
  the parent omit the submodule entirely. The descent would then simply never
  happen, and a manifest could be sealed over unrecorded submodule edits — a
  regression against the Linux writer's old code, which scanned `build` and `v8`
  directly. Submodules are now enumerated from the index (`git ls-files --stage`,
  mode `160000`) and each is scanned unconditionally, with `--ignore-submodules=all`
  on the parent scan to say explicitly that the parent's view of them is never
  relied upon. Proved on a fixture that first asserts the bypass is real — the
  parent's status genuinely omits the dirty submodule — and then that the proof
  still reports `sub/inner.txt is not HEAD plus the declared patches`.

  The "is each declared patch applied" probe was **removed** in the same pass. Once
  every declared path is part of the replay it is redundant, because the comparison
  cannot pass unless each patch is present; and it actively misreported, because an
  edit made *beside* a declared change in the same file breaks that patch's
  reverse-applicability, so the probe blamed the patch instead of naming the file
  that had drifted. Both facts are fixtures.

  `scripts/ci/tests/test_v8_source_proof.py` now covers the shared module directly
  in 10 tests: an exactly-patched tree spanning a submodule, an unapplied patch, the
  ignore-config bypass, an edit beside a declared change in the same file, an
  undeclared path, an exemption that applies only when named exactly, a submodule
  moved off its pinned commit, a zero-patch declaration requiring a pristine tree,
  a flipped executable bit, and a patch that creates a nested file. Re-sealing after
  the restructure produced the same `component_id`s, so what the manifests record did
  not change.

  A second review round found three more, all real:

  A patch that **creates** a nested file was rejected. GNU patch will not create
  missing directories, and the materialisation skipped `mkdir` when there was no HEAD
  blob to write, so a valid exactly-patched tree failed to replay. The real
  `0008-ohos-toolchain.patch` is exactly this shape — its only target is
  `build/toolchain/ohos/BUILD.gn` — and it is now a fixture.

  The new test file **was not running in CI**. Both `pr-ci.yml` and `release.yml`
  enumerated Python test scripts by hand, so the submodule-ignore regression guard
  would never have executed. Both now use `unittest discover`, which also revealed
  the list had already drifted twice before: `test_collect_render_metrics.py` and
  `test_compare_render_results.py` were never run either. Discovery runs 81 tests
  across 8 files where the list covered 5.

  The Linux writer's submodule test **passed without exercising submodule descent**.
  Its `build` and `v8` were nested checkouts never registered as gitlinks, so
  `direct_submodules` did not descend and the parent's bare `?? build/` satisfied a
  loose regex. They are now registered with `update-index --cacheinfo 160000` and the
  assertion demands the nested `build/config/c++.gni` path, so the test fails if
  descent stops working.
- [ ] 1.1g Record how the binding was obtained in the manifest. The schema uses
  `deny_unknown_fields` and has no field for binding origin, so a manifest sealed
  after reusing a verified prebuilt binding is indistinguishable from one sealed
  after regenerating it from source. Add the field and populate it. Until then the
  origin exists only in build logs, which are not shipped.
- [ ] 1.1h Treat a version-only libclang gate as insufficient. Chromium's
  `third_party/rust-toolchain/lib/libclang.so` reports clang 22 but silently emits
  a **wrong** binding: `cppgc_Visitor` sized 1, nested enums missing their
  `v8_String_` prefixes, 840 items instead of 870, and four compile errors
  including a `1_usize - 8_usize` overflow. It ships no sibling `bin/clang`, so
  build.rs's `-print-resource-dir` probe finds nothing and bindgen falls back to
  NDK clang 12's builtin headers. A misconfigured libclang does not fail loudly,
  it corrupts the FFI ABI, so the gate must also require a usable sibling
  `bin/clang` and the regenerated binding must be diffed against the recorded one
  before it is accepted.
- [x] 1.2 Add one repository release-version source under `release/` and verify
  propagation to Cargo, Gradle, CMake, the HAR, archives, manifests, examples,
  and documentation.

  **Done 2026-08-09.** `release/VERSION` is the source; every consumer reads it and
  `scripts/test-release-version-contract.sh` (in `pr-ci.yml`, so also in
  `verify-change.sh`'s contract lane) is what makes that true rather than intended.

  **Four build systems had four answers, and two were wrong in ways that ship.**
  The Android AAR reported `0.9.0` while the Android C-API SDK built beside it
  defaulted to `0.1.0` — one platform disagreeing with itself, because two scripts
  build the two artifacts and only one had been told. The Windows SDK read a
  version from `crates/capi/Cargo.toml` and then *discarded* it, hardcoding `0.1.1`,
  so it announced a version no other platform had heard of. Linux and HarmonyOS
  derived theirs from that same crate manifest, HarmonyOS with a silent `0.1.0`
  fallback. A build that stops is recoverable; an archive labelled with a version
  nobody chose is not, so every reader now refuses instead of defaulting.

  **Plain text, and the reason is not taste.** All four toolchains must read it
  with no dependency: bash, Python, Gradle, and CMake 3.16, which has no JSON
  parser. A JSON source would need `jq` in bash and a parser bump in CMake.

  **Cargo is the one mirror.** A manifest takes a literal, so `[workspace.package]
  version` cannot read a file; the gate holds the two equal instead. Sixteen
  per-crate literals collapsed into it via `version.workspace = true`, and
  `Cargo.lock` moved 16 version lines and nothing else — checked, because CI runs
  `cargo fetch --locked` and a stale lock fails there rather than here.

  **The Windows literal's reasoning is kept, not deleted.** `0.1.0` shipped a DLL
  that could attach no surface kind and the fix went out as `0.1.1` so those bytes
  would not arrive under a version a consumer already held. A single forward-moving
  source honours that; a detached per-platform literal is what lost it.

  **Left independent deliberately, and checked so it cannot be folded in later:**
  `platforms/openharmony/entry/` + `AppScope/` are a demo *application*
  (`"type": "entry"`, `com.migo.ohoshost`, vendor `example`), not the shipped
  library; `tests/c_host/android/` is a test application; `MIGO_ABI_VERSION_*` in
  `include/migo/types.h` answers a different question from which release a binary
  came from; `adapter/` is a separately publishable npm package.

  **The version stays at the value already shipped in the AAR.** Nothing here
  proposes a new one — the `0.10.0-rc.1` bump is now a single edit, which is the
  point.

  Proved end to end and by injection. `release/VERSION` set to `0.9.1-probe`
  reached the generated `BuildInfo.java`, and was restored. The shared bash reader
  was extracted and exercised on four inputs — normal, padded, empty, missing —
  because three of the four SDK scripts cannot run on this machine. Ten gate
  checks go red: the Cargo mirror drifting, a crate reintroducing its literal, the
  workspace losing `[workspace.package]`, Gradle re-hardcoding `versionName`, a
  packaging script re-hardcoding, the HarmonyOS fallback returning, a non-semver
  source, stray whitespace in the source, the demo application being folded in,
  and the source missing entirely.

  **Two of those ten passed at first, and both were the gate's fault.** The marker
  check looked for the reader's *name*, which also appears in the reader's own
  definition, so a call site replaced by a literal left the name behind and the
  gate stayed green; the marker is now the assignment, searched with comments
  stripped. And the literal scan's lookbehind excluded a preceding `-`, so
  `${MIGO_VERSION:-0.1.0}` — the exact fallback being forbidden — did not match.
  Neither would have been found by reading the gate.

  Verified: `cargo metadata` (16 members, all `0.9.0`), `cargo check --locked`,
  Java 207 per flavour in both profiles, the android-host-api gate, and
  `test-local-verification-contract.sh` at 27 derived gates.
- [ ] 1.3 Materialise V8, Skia, and ANGLE under immutable content-addressed
  paths before any Cargo or native link, covering the HarmonyOS path that
  currently builds V8 from an external checkout.
- [ ] 1.4 Remove every `--allow-multiple-definition`, including both HarmonyOS
  target entries in `engine/.cargo/config.toml`, and resolve duplicate C++
  runtime symbols at component build boundaries.
- [ ] 1.5 Make release metadata and archives deterministic under
  `SOURCE_DATE_EPOCH` on all four platforms.
- [ ] 1.6 Repair Android PowerShell packaging and reject release `--skip-rust`.
- [ ] 1.7 Replace the Windows warm-target link flow with a clean Windows-native
  MSVC and ANGLE build graph.
- [ ] 1.8 Implement HarmonyOS audio playback through OHAudio.
- [ ] 1.9 Add the HarmonyOS V8 component manifest binding the shipped archive to
  a source revision and GN argument set. The current HarmonyOS package manifest
  records this absence as a known gap.
- [ ] 1.10 Prove the HarmonyOS API floor with the two-sysroot symbol audit
  (`MIGO_OHOS_FLOOR_SYSROOT` at the floor plus `MIGO_OHOS_NEWER_SYSROOT`), set
  `compatibleSdkVersion` to the proven floor, and record any symbol that forces
  the floor higher.

  **Run 2026-08-09, and it needed no installation: the newer SDK was already
  unpacked at `~/ohos-sdk-6.1` (6.1.0.31, API 23) beside the floor at
  `~/ohos-sdk` (5.1.0.107, API 18).** Three earlier notes in this session asked a
  human to download it. Checking `ls ~/ohos-sdk*` before asking would have
  answered that; the obstacle was recorded rather than verified, which is the
  shape this ledger keeps finding and this time produced it.

  ```
  MIGO_OHOS_NEWER_SYSROOT=$HOME/ohos-sdk-6.1/native/sysroot \
    MIGO_OHOS_TRIPLE=<triple> bash scripts/test-ohos-symbol-floor.sh \
    dist/migo-ohos-<arch>/lib/libmigo_capi.a
  ```

  Both architectures pass with a real comparison behind them: the floor exports
  8,200 symbols across 117 libraries, the newer sysroot 10,135 across 132, so
  **1,935 symbols postdate the floor** and `libmigo_capi.a` imports none of them
  (x86_64: 645 undefined, 516 floor-resolved; aarch64: 637 and 512).

  **The gate was improved before it was believed.** Its two-sysroot branch printed
  nothing on success, so "compared 1,935 candidates and found none" was
  indistinguishable from "compared an empty set" — an absent newer sysroot with
  the right directory name would have passed every artifact silently. It now
  states the delta it compared and **fails closed** when that delta is zero.
  Falsifiable: pointed at an empty directory it reports
  `newer sysroot … adds no symbol over the floor (0 exported); a comparison with
  nothing to find is not evidence` and exits 1.

  **Still open before this can be `- [x]`:** it is not wired into any gate — the
  variable is opt-in and `build-ohos-sdk.sh` does not set it — and neither
  independent review has run.
- [ ] 1.11 Produce deterministic Android, Linux, Windows, and HarmonyOS packages
  carrying manifests, checksums, BSL 1.1 text, notices, SBOMs, and provenance.
- [ ] 1.12 Prove same-source rebuild byte equality for every shipping archive.
- [ ] 1.13 Verify the Android permission contract from the built Full and Slim
  merged manifests at API 26, 28, and 31.
- [ ] 1.14 Package the HarmonyOS HAR reproducibly with the unified version.

