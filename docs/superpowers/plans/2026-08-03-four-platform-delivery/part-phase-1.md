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
- [x] 1.1l Wire or delete `required_patches` in the Windows V8 lock. **Wired
  2026-08-09.** The lock declared three bare id strings that nothing read, while
  `scripts/build-v8-windows.sh:196` named its own three globs — the same
  declared-in-two-places shape task 1.1b removed from Android, in the other direction.
  The lock now carries the `id`/`file`/`notes` entries, and the build reads it.

  **The reader is shared rather than copied.** Android had the lock parser inline; a
  second copy in the Windows script would be the drift this item exists to remove, so
  it moved to `v8_read_declared_patches` in `scripts/lib/v8-patch-apply.sh`, which both
  scripts already source. Verified against both locks: four entries for Android, three
  for Windows, and `test-v8-patch-application-contract.sh` now counts 1 patch declared
  as a script literal (OpenHarmony's) and **7 in locks**, with all 8 still resolving to
  exactly one file each. The dual-source guard is what makes that safe: Windows'
  literals leaving the scripts would otherwise have narrowed the check silently, which
  is exactly how it once went from 8 to 4.

  **Not done, and it needs a Windows run:** `write-windows-v8-component-manifest.py`
  still records no patch provenance, so a Windows manifest does not attest which patches
  produced it the way the Android one now does. The field is no longer decorative —
  the build enforces it — but the manifest half is open and belongs with 1.1g/1.1j.

  **Doing this exposed a real defect in the shared checkout, found by the contract test
  going red mid-session while an OpenHarmony V8 build was running.** One vendored
  `rusty_v8_src` serves all four platforms, and `0008-ohos-toolchain.patch` *creates*
  `build/toolchain/ohos/BUILD.gn`, which the Android declaration does not touch — so
  building OpenHarmony made the Android replay refuse a file that **is** explained by a
  committed patch, just not by one that build applies. The two platforms were mutually
  exclusive on one checkout, and the earlier note under 1.1i predicted this before it
  happened.

  `--accounted-patch <glob>` answers it inside the existing mechanism: the accounted
  paths are **derived from the patch** rather than listed, so they cannot drift from
  what it creates, and only paths a patch *creates* may be accounted for — accounting
  for one a foreign patch merely *modifies* would skip content verification on a file
  this platform's own patches may also touch, so it is refused. Three fixtures: the
  foreign-created file is refused when not accounted for, accounted for when it is, and
  a modify-only patch cannot grant an accounting at all (checked against an otherwise
  exactly-patched tree, so the refusal can only come from that guard).

  Second defect from the same red run, and simpler: `build-v8-ohos.sh` wrote its build
  log **into** `$RUSTY_V8_SRC`, where it is an untracked file no patch explains, so it
  broke every provenance gate over that tree. `build-v8-android.sh` already wrote its
  log under `$TMPDIR`; the OpenHarmony script now does the same.

  **Independent review found six issues, and one was a regression this change
  introduced.** `V8_ACCOUNTED_ARGS` is forwarded to
  `write-v8-component-manifest.py` as well as to the shell proof, and that parser knew
  only `--accounted` — so every Android V8 build would have died on
  `unrecognized arguments: --accounted-patch` **after** the expensive build. Separating
  the arrays would only have moved the failure, because the writer runs the same replay
  proof and so needs the same accounting; `accounted_paths_from_patch` now lives in
  `scripts/lib/v8_source_proof.py` and enforces the identical create-only rule. Verified
  against the real tree: the option is accepted, the proof passes, and
  `--accounted-patch 0002-install-sysroot.patch` is refused with
  `creates no file, so it cannot account for one`.

  **And the recipe hash is verified, so editing a V8 build script is not a cosmetic
  change.** `generate-android-artifact-manifests.py` compares
  `provenance.build_recipe_sha256` against the script's current bytes, so both committed
  component manifests became **wrong rather than stale** the moment
  `build-v8-android.sh` changed — every AAR build failed with
  `V8 build recipe bytes hash mismatch`. That correction matters: the earlier judgement
  here, borrowed from 1.1b's patch-set note, was that nothing validates such staleness.
  Both manifests were regenerated from warm builds rather than edited, which also
  produced a **second reproducibility data point**: `aarch64` came back at
  `681aaa39…` and `x86_64` at `ce14223a…`, both matching their recorded hashes, with new
  ids `3a5841af…` and `21613b7e…` recording recipe `62d717ff…`.
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
- [x] 1.1i Verify the aarch64 archive is unchanged now that patch 0002 applies.
  **Re-run 2026-08-09: sha256 `681aaa39367a9aa35ab7e584ddd4b36273acbc0ccb4177648c43b9b55b7eb273`,
  unchanged.** Task 1.1e's reasoning holds, and the run shows the mechanism rather
  than only the conclusion: patch 0002 is in effect, so the Debian arm64 sysroot
  **was** downloaded (`chrome-linux-sysroot/2f915d82…`), `gn gen` re-ran, and the
  regenerated `args.gn` carries `use_sysroot = true` **twice** — the duplicate GN
  assignment 1.1e predicted, which GN accepts. `ninja: no work to do` followed, which
  is the actual evidence: the regenerated build graph is identical to the previous
  one, so `sysroot.gni` testing `is_android` before `is_linux && use_sysroot` really
  does keep an Android toolchain off the Debian sysroot. The binding is byte-identical
  to the verified prebuilt input, and re-sealing produced a **byte-identical**
  `component-manifest.json` (`b5299a05…`, `component_id` `ee0fb437a33fcd9c…`), which
  independently confirms 1.1f's determinism fix on a live re-seal.

  **Scope of the claim, stated because it is narrower than "the archive reproduces":**
  this was a warm build, so no C++ was recompiled. What is proved is that applying
  0002 does not change the build graph. From-scratch reproduction of both
  architectures is task 1.1's evidence, not this one's.

  **Running it found two defects in the gate 1.1c and 1.1d built, and the first made
  the Android V8 build impossible on this machine.** `_v8_git` passed
  `-c safe.directory=$tree` — but git compares that value *literally* against the
  repository path it discovers, and every caller derives the tree as
  `$PROJECT_ROOT/../rusty_v8_src`. The unnormalised `..` never matches, so the
  exception did not apply, every git call failed `dubious ownership`, and the replay
  reported **"the rusty_v8 tree carries changes the committed patches do not
  explain"** — an accusation about the tree for what was a refusal to read it. The
  path is now canonicalised at that single choke point, which also covers the
  submodule descent and any caller's `RUSTY_V8_SRC`. Note the contract test could
  never have seen this: it derives its own `real_tree` through `cd .. && pwd`, so the
  test normalised what the build script did not.

  Second, the failure was **fail-open**. `_v8_changed_paths` read `git status` through
  a process substitution and the enumeration was read through another, so neither
  exit status was observable and a git failure arrived as "no changed paths". With a
  declared patch that only *creates* a file — the shape of `0008-ohos-toolchain.patch`
  — the replay then succeeds into the scratch directory, the byte comparison has
  nothing to iterate, and the function **certifies a tree it never managed to read**.
  Both producers now propagate. Two mutants, each killing exactly one of the two new
  contract cases at its own assertion: dropping the canonicalisation kills "a checkout
  this user does not own is read through a path carrying ..", and restoring the process
  substitution kills "a tree whose git status fails is refused, not read as unchanged".

  **Recorded, not fixed: the vendored checkout can only be in one platform's declared
  state at a time.** `build-v8-ohos.sh` applies 0008 to the same
  `../rusty_v8_src` and never reverts it, and 0008 is not in the Android declaration,
  so an OpenHarmony V8 build leaves the tree in a state the Android replay refuses.
  `build-v8-ohos.sh` does not call the replay at all, so it does not notice. Either
  each platform accounts for the others' patches or the OpenHarmony build needs its
  own checkout; until then, run the Android build before the OpenHarmony one.
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
- [x] 1.1h Treat a version-only libclang gate as insufficient. **Done 2026-08-09**,
  and the object the item names was checked before the fix was written:
  `rusty_v8_src/third_party/rust-toolchain/lib/libclang.so` is a symlink chain to
  `libclang.so.22.0.0git` and `clang_getClangVersion()` really does report
  `clang version 22.0.0git`, so it passes any version floor — while its sibling `bin/`
  contains `bindgen`, `cargo`, `rustc` and **no `clang`**. A misconfigured libclang does
  not fail loudly; it corrupts the FFI ABI.

  Two guards, because the two failure modes are independent. Acceptance now also
  requires a sibling `clang` (or `clang++`) whose `-print-resource-dir` resolves to a
  directory that exists — the exact fact whose absence makes bindgen fall back to
  another toolchain's builtin headers. Falsifiable, and run against the real objects:
  Chromium's `rust-toolchain/lib` is **rejected**, while NDK 23's libclang is accepted
  with resource dir `.../lib64/clang/12.0.9` (and is then still refused by the version
  floor, so the two checks compose rather than overlap).

  And a regenerated binding is now diffed against the one the component manifest
  records. A difference is either a real V8 ABI change or a bad libclang, and the bytes
  alone cannot tell them apart, so the build stops instead of sealing a manifest over an
  FFI surface nobody compared. `MIGO_V8_BINDING_CHANGE_EXPECTED=1` is how a V8 bump says
  the difference is the point, which makes accepting a new ABI deliberate rather than
  incidental.

  **Not yet exercised end to end:** the refusal path was verified as a function against
  both real libclang directories, not by a full build with `V8_LIBCLANG_PATH` pointed at
  Chromium's — an OpenHarmony V8 build held the shared `rusty_v8_src` cargo target
  directory, and two cargo builds in one target directory serialise on its lock. Run
  `V8_LIBCLANG_PATH=$RUSTY_V8_SRC/third_party/rust-toolchain/lib
  scripts/build-v8-android.sh aarch64` once that is free and confirm it falls back to
  the prebuilt binding rather than regenerating.
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

  **Metadata half done 2026-08-09. The archive half cannot be done yet, and that is
  a finding rather than a delay: nothing in this repository creates a release
  archive.** The four SDK scripts populate a prefix *directory*
  (`$PREFIX/lib/cmake/migo/...`); the only archive that exists today is the AAR,
  which Gradle builds. Item 1.11 is what produces packages, so archive determinism
  is a property of code that is not written. `scripts/lib/reproducible-timestamp.sh`
  exists so 1.11 has the stamp to use rather than inventing a third one.

  **Three shipped or committed artifacts recorded when they were built.** One wall
  clock anywhere in the set defeats 1.12 for the whole release:

  * `build-aar.sh` wrote `"sourceDateEpoch": <the epoch>` and then
    `"buildTime": "<local wall clock>"` **on the next line** — the input for
    reproducibility recorded and unused for the one field that broke it. Local, not
    UTC, so the same source differed between two timezones as well as between two
    minutes. `build-aar.ps1:225` did the same on Windows.
  * `generate-sbom.sh` stamped `metadata.timestamp`, and `release.yml:376` writes
    that SBOM into the Android dist directory, so it is a release asset.
  * `write-snapshot-manifest.sh` stamped `generated_at` into the manifests under
    `engine/crates/runtime-v8/snapshots/`, which are **committed** — three of them
    currently carry distinct 2026-08-06 wall clocks. Every regeneration diffs on the
    timestamp alone, and a tracked file that cannot be reproduced from the sources
    it describes is not a manifest of anything. Those values will settle the next
    time the snapshots are regenerated; nothing here rewrites them, because the
    aarch64 round needs a device.

  All three take `SOURCE_DATE_EPOCH` when set and a wall clock when not, always
  UTC, and refuse a malformed value rather than treating it as absent — a caller
  that set it believes it is producing something reproducible.

  **Verified by measurement, not assertion.** The SBOM is byte-identical across two
  runs a second apart under a fixed epoch (`08c878bf…`) **and changes when the
  epoch changes** (`ff41012d…`). The second half matters more than the first: bytes
  that are merely stable prove the timestamp was removed, not that it tracks the
  epoch. The bash helper was exercised on four inputs — absent, fixed, zero,
  malformed. `build-aar.ps1` is inspection-only: there is no `pwsh` here, so it was
  written to assume as little as possible — self-contained rather than reading an
  outer-scope variable, and one expression per line, because PowerShell ends a
  statement at a newline and leading-dot continuation is a C# habit that does not
  parse.

  `scripts/test-reproducible-timestamp-contract.sh` holds the rule: a script under
  `scripts/` that reads a clock must name `SOURCE_DATE_EPOCH` or be listed as
  producing something that does not ship. The list is per file with a reason each
  rather than a directory exclusion, and a **stale exemption is an error** — one
  that no longer applies grants more than it needs to. Both directions were proved
  red: a reintroduced wall clock, and an exemption left behind after its clock was
  removed. A stopwatch and a build stamp look identical to a regular expression, so
  which is which has to be stated by a human.

  **What remains for 1.5:** archive determinism (sorted entries, fixed mtime, no
  gzip timestamp, fixed uid/gid) on all four platforms, which is 1.11's code, and
  the Windows and HarmonyOS packaging paths cannot be run on a Linux workstation.
- [x] 1.6 Repair Android PowerShell packaging and reject release `--skip-rust`.

  **Done 2026-08-09.** The `--skip-rust` half landed earlier; what remained was the
  PowerShell entry point, and the reason it was still broken is that it could not be
  run: it probed only for `gradlew.bat`, so it was unrunnable under the `pwsh` that
  exists on Linux. Both wrappers are committed, so it now selects by host — and every
  finding below came from actually executing it.

  **The serious defect was an unpinned NDK, not the packaging.** Task 1.1a pinned the
  NDK so "an AAR can no longer be built with a different NDK than the V8 archive it
  links"; `build-android-so.ps1` tested only that `$env:ANDROID_NDK_HOME` was
  non-empty and used whatever it named. The NDK supplies the compiler, sysroot and
  linker that the component manifest records, so a Windows build could link the pinned
  V8 archive with any toolchain and be stamped as if it had not. It survived because
  the enumeration meant to prevent exactly this globs `*.sh`
  (`test-android-ndk-pin-contract.sh:137`) — the gate's *scope*, not its assertions.
  `scripts/lib/AndroidNdk.psm1` is the PowerShell counterpart of
  `scripts/lib/android-ndk.sh`: same lock field, same candidate order, selection by
  the NDK's own `Pkg.Revision`, and a rejected override is reported as it happens
  rather than only when nothing matches, because falling through in silence is the
  substitution the pin exists to prevent.

  The gate is behavioural rather than a grep for `Pkg.Revision`, which occurs in the
  module's own prose and would pass over a module that had stopped reading it. The
  discriminating case is a directory named **exactly** like the pin whose
  `source.properties` reports `1.2.3456789`: refused. An NDK reporting the pin is
  accepted. Mutation: replacing the resolver call with `$env:ANDROID_NDK_HOME` makes
  the enumeration fail `build-android-so.ps1 uses ANDROID_NDK_HOME without resolving
  it`.

  **The packaging defect was that a release could not succeed at all.** The ps1 never
  staged the verified package index and never passed
  `-PmigoVerifiedReleasePackaging`, so `verifyMigoReleaseArtifactPackaging<Profile>`
  refused with a message naming `scripts/build-aar.sh` — a bash script, to a Windows
  user, after the Rust build and the Gradle clean. It now takes `-ArtifactManifest`
  with the shell twin's policy (release means `required`, refused otherwise, at
  argument time), stages through the **same** generator, metadata writer and Rust tool
  rather than reimplementing what a manifest says, and verifies the produced AAR
  against the index afterwards.

  **Proved by comparison, which is what specification §7.4 asks for.** Under a fixed
  `SOURCE_DATE_EPOCH`, a debug AAR staged through each entry point produced
  byte-identical inputs — `package-index.json` `59b5b7b3…`, `slices/arm64-v8a.json`
  `e7cffba7…`, `build-metadata.json` `1297b7f6…`. Then the case that was previously
  impossible: `pwsh -File scripts/build-aar.ps1 -BuildType release …` ran
  `:library:verifyMigoReleaseArtifactPackagingFull` and **passed** it. Staging
  correctness needs no separate assertion, because that Gradle task is what reads the
  staged inputs, so a script staging the wrong thing fails there.

  **One bash defect found on the way, introduced with the refusal itself.**
  `--unverified-native-libs` shifted inside its own `case` *and* at the loop tail, so
  it silently swallowed the following argument: an explicit `arm64-v8a` disappeared
  and the build widened to every ABI. The pre-existing gate case could not see it —
  what it swallows there (`release`) is a valid positional and also the default — so
  the new case passes an argument that is not (`--artifact-manifest off`), which the
  bug turns into `Unknown argument: off`. The three argument-time refusals are now
  asserted through both entry points; the ps1 half skips when `pwsh` is absent.

  **Independent review found four more, all real, and two of them meant a PowerShell
  release was still not the shell's package.** (a) `$IsWindows` does not exist in
  Windows PowerShell 5.1, where it evaluates false — so on the one host this entry point
  exists for, it would have chosen the Unix `gradlew` and an extensionless manifest tool.
  `#Requires -Version 7.0` makes that unrepresentable instead of adding a second
  host-detection rule to keep in agreement. (b) It never produced the external
  `<aar>.attestation.json` sidecar, and never removed a stale one, so a successful
  release left an attestation describing different bytes or none at all; it now runs the
  same `attest` and `verify-attestation` steps and clears the outputs first. (c) The
  resolver omitted Windows' documented `%LOCALAPPDATA%\Android\Sdk`, so a normal Android
  Studio install could hold the pinned NDK and still be rejected. (d) A single-ABI build
  wrote `migo-full-release.aar` where the shell writes
  `migo-full-release-arm64-v8a.aar`, so the two overwrote each other under one name —
  found here rather than by the review, and the per-ABI figure is the one a host weighs
  against its APK budget.

  Verified after the fixes: `pwsh -File scripts/build-aar.ps1 -BuildType release …`
  completes, passes `verifyMigoReleaseArtifactPackagingFull`, and writes
  `migo-full-release-arm64-v8a.aar` beside `….aar.attestation.json`.
- [ ] 1.7 Replace the Windows warm-target link flow with a clean Windows-native
  MSVC and ANGLE build graph.
- [x] 1.8 Implement HarmonyOS audio playback through OHAudio.

  **Done 2026-08-10, and the OpenHarmony package now ships the full product profile
  instead of `profile-slim`.** That substitution was the whole cost of the gap: the audio
  crate reached a device only through cpal, cpal has no OHAudio backend, and pulling it in
  for OpenHarmony dragged `alsa-sys` and `pkg-config` in for a library the platform does
  not have — so the package was built slim and shipped no audio at all.

  **The seam is the device half only.** `AudioSync`, the watermarks, the ring and
  `OutputCallback`'s real-time render logic were already independent of cpal, so they stay
  in `output.rs` and both backends use them; `output_cpal.rs` and `output_ohaudio.rs` hold
  nothing but device setup. That matters beyond tidiness: Section 7.3's steady-state and
  first-call allocation gates are written against `OutputCallback`, so they now cover the
  OpenHarmony callback without being duplicated — a second copy of a real-time callback is
  how one of them quietly stops matching its gate.

  **Declared by hand against the SDK headers, ten functions and six enum constants.** The
  callback struct is passed **by value** to `OH_AudioStreamBuilder_SetRendererCallback`, so
  all four members are declared even though two are used: a short struct passed by value
  reads the caller's stack as if it were the rest. `length` is a byte count and the stream
  is `F32LE`, so a length that is not a whole number of samples is refused rather than
  rounded — rounding would leave the tail of a device buffer undefined. The callback state
  is a box freed **after** `OH_AudioRenderer_Release`, because the device thread reaches it
  through `user_data` and freeing it earlier is a use-after-free on a real-time thread.

  **Evidence, in the order it was obtained:**

  * `migo-capi` compiles for `x86_64-unknown-linux-ohos` with **default features**, which
    is what pulls the audio crate. That build did not exist before.
  * All fourteen `OH_Audio*` functions appear as undefined imports in the archive and
    **all fourteen resolve** against the sysroot's `libohaudio.so`.
  * The staged package's own contract links an **external consumer** against it with the
    package's link list (now including `ohaudio`) and every `migo_*` resolves — a real
    link, not a symbol table read.
  * The API floor is unchanged: externally undefined went 645 → 660 and floor-resolved 516
    → 531, the same +15, so every new import is satisfied at API 18 and audio did not
    raise `ohos_api`.
  * The host path is untouched: `migo-audio` still passes 65 tests including both
    allocation gates, and the slim profile still builds.

  **A tooling trap worth keeping.** The first symbol check reported *no* OHAudio imports,
  and that was a lie: Rust emits LLVM bitcode members and the SDK's `llvm-nm` 15 cannot
  read LLVM 22, so it printed nothing for every query. The control that caught it —
  running the same query against an artifact known to contain audio — showed zero as well,
  because that one is stripped. Reading a tool's silence as an answer needs the tool to be
  able to see the thing first.

  **Not verified, and it needs a device:** nothing has been *heard*. Playback, latency
  under `AUDIOSTREAM_LATENCY_MODE_FAST`, interrupt handling and route changes are all
  emulator or hardware work, which is item 2.5's audio row. What is proven here is that
  the code exists, compiles for its own target, and links against the platform library.
- [x] 1.9 Add the HarmonyOS V8 component manifest binding the shipped archive to
  a source revision and GN argument set.

  **Done 2026-08-10** for `x86_64`, the architecture whose archive exists. The gap
  `scripts/build-ohos-sdk.sh` recorded in its own package manifest — "no component
  manifest binds this archive's embedded V8 to a source revision and GN argument set" —
  is removed, because it is no longer true.

  Four pieces, each mirroring what Android and Linux already do rather than inventing a
  fourth arrangement: `contracts/artifact-manifest/ohos-v8.lock.json` (the single
  declaration of revisions, patch set, SDK pin and per-arch target shapes), a
  `("linux", "ohos")` arm in the Rust validator, `write-ohos-v8-component-manifest.py`,
  and a seal step in `build-v8-ohos.sh`. Sealed and verified:
  `component_id` `78fa8c66…`, and `migo-artifact-manifest verify-v8-component` passes
  against the real 172,812,984-byte archive and its binding.

  **`linux`/`ohos`, not `os = "ohos"`.** That is the pair the compiler reports
  (`target_os = "linux"`, `target_env = "ohos"`) and the pair `capi/src/platform/mod.rs`
  selects on, so a third spelling would be a fact about nothing. The floor is
  `ohos_api = 18`: what V8 was *compiled against*, i.e. the SDK's own sysroot, not the
  higher product floor `build-ohos-sdk.sh` declares to consumers — those are different
  claims and both are now recorded where they belong.

  **The writer reproduced two of 1.1f's defects before they were fixed, which is why
  that entry's lessons were worth reading first.** `rustc --version` run from the
  repository root reported **1.93.0** while the vendored checkout pins **1.89.0** —
  recording a compiler that had built nothing in it — so it is now resolved with the
  working directory inside that checkout. And `clang --version` embeds its
  `InstalledDir`, which for this platform is Chromium's clang *inside* the vendored
  tree, so the manifest carried an absolute machine path; it is normalised to
  `${RUSTY_V8_SRC}`, as the GN args already were to `${OHOS_NDK_HOME}`. Verified by
  sealing twice: byte-identical.

  **The archive now verifies through the same path as every other target.**
  `bash scripts/fetch-v8-archives.sh --check --all` reports four archives verified
  including `x86_64-linux-ohos`, so the fetcher's own rule — "a target with no committed
  manifest cannot be fetched, by design" — is satisfied rather than worked around.
  `aarch64-linux-ohos` is deliberately still absent: no archive, so nothing to verify
  against.

  **It also runs the replay proof, which no OpenHarmony build had ever done**, so a
  stray edit in the shared checkout could previously have reached this archive
  unrecorded. Android's declared patches are accounted for by path here — the mirror
  image of `--accounted-patch` on that side — because one checkout serves both and each
  platform's patches touch files the other does not declare.

  **A drift found on the way, and the reason it went unnoticed for so long:** the
  committed JSON schemas are enforced by nothing. `v8-component-schema-v1.json` listed
  Android and Linux under `target` while the Rust validator had been accepting Windows,
  so a Windows manifest was valid to the tool and invalid to the document describing it.
  Both are now listed, and `test-artifact-manifest-contract.sh` equates the schema's
  target set with the validator's arms — checked load-bearing by deleting the
  OpenHarmony entry, which fails naming `validator-only targets: [('linux', 'ohos')]`.
  All four committed manifests were also validated against the schema with `jsonschema`
  as one-off evidence; that is deliberately not a permanent gate, since the drift to
  prevent is between the two statements rather than inside either one.

  **Still open here:** `aarch64` needs its own archive (and a Skia) before it can be
  sealed, and the Windows writer still records no patch provenance — see 1.1l.
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

  **The wiring half is done — 2026-08-10.** `build-ohos-sdk.sh` now runs the floor gate
  as part of a package build and *discovers* the newer sysroot instead of waiting to be
  handed one, so the post-floor comparison stops being a check nobody runs. Selection is
  on the SDK's own declared `apiVersion`, not on its directory name, and the first draft
  taking "the highest-sorted directory that is not the floor" was **wrong**: with the
  floor as the newest installed SDK it hands back an *older* sysroot and reports its
  extra symbols as post-floor, which is evidence pointing the wrong way. Three layouts
  are checked — a newer and an older SDK present (picks the newer), the floor being the
  newest (refuses, reports none), and only an older one beside it (refuses).
  `MIGO_OHOS_NEWER_SYSROOT` still wins when set.

  On this machine only one SDK exists, so the run says so —
  `no second OpenHarmony SDK found, so only the floor half of the API gate runs` — rather
  than passing silently, which is the T.8 reporting rule applied here.

  Measured on the freshly staged `x86_64` package: floor exports 8,200 symbols across
  117 libraries, `libmigo_capi.a` has 645 externally undefined of which 516 are resolved
  by the floor, and no import postdates it. Those are the same numbers this entry
  recorded, now reproduced locally.

  **Still open:** the two-sysroot delta itself cannot be demonstrated here until a second
  SDK is unpacked, `compatibleSdkVersion` has not been set from the proven floor, and
  neither independent review has run.

  **Correction, 2026-08-09: the evidence above was not produced on this workstation
  and cannot be reproduced here yet.** `ls -d ~/ohos-sdk*` finds only `~/ohos-sdk`
  (5.1.0.107, API 18); there is no `~/ohos-sdk-6.1`, and `dist/` does not exist, so
  neither the newer sysroot nor the `libmigo_capi.a` the run measured is present. The
  entry's own lesson — check the object before believing a claim about it — applies to
  the claim itself. Reproducing it here needs the 6.1 SDK downloaded and an
  OpenHarmony build, which is blocked behind 1.9.
- [ ] 1.11 Produce deterministic Android, Linux, Windows, and HarmonyOS packages
  carrying manifests, checksums, BSL 1.1 text, notices, SBOMs, and provenance.
- [ ] 1.12 Prove same-source rebuild byte equality for every shipping archive.
- [x] 1.13 Verify the Android permission contract from the built Full and Slim
  merged manifests at API 26, 28, and 31.

  **Done 2026-08-09**, and the gap was narrower than the wording suggests, because
  `test-permission-coverage-contract.sh` already holds the *source* manifests to a
  policy table including each `maxSdkVersion`. Two things it does not do, both of which
  are what "built ... merged" means: it reads
  `library/src/{full,main}/AndroidManifest.xml` rather than what Gradle produces, so a
  permission contributed by a dependency or a `tools:` directive is invisible to it;
  and it iterates the policy, so it checks every expected permission is **present** and
  never that an unexpected one is **absent**. A `READ_CONTACTS` added to the Full
  manifest passed it.

  `scripts/test-android-merged-manifest-permissions.sh` compares the merged manifests
  exactly, in both directions, per profile, and is in `pr-ci.yml` — so
  `verify-change.sh` derives it too (confirmed: it appears in `--plan-only`, and
  `test-local-verification-contract.sh` passes).

  **The policy is not restated.** It moved to
  `scripts/lib/android_permission_policy.py` and both gates import it; the older gate
  still reports the same 30 gated / 8 cleanup / 38 sensitive ops, so the extraction
  changed nothing it asserts. A second copy of that table is the "two implementations
  of one rule" shape, where the tests end up over the one that never ships.

  **The debug variants are used, and why that is evidence about the release artifact.**
  `processFullReleaseManifest` depends on `verifyMigoReleaseArtifactPackaging<Profile>`,
  which refuses unless a release build has staged verified inputs, so requesting it
  would gate on whatever an earlier run left behind. Instead the gate asserts no
  build-type source set declares a manifest (`src/debug`, `src/release`: neither
  exists) *and*, whenever a release merged manifest is present, that it is identical to
  the debug one. On this run it was: `fullDebug` and `fullRelease` merged manifests are
  byte-identical.

  **The contract, measured rather than derived by hand:**

  | Profile | API 26 | API 28 | API 31 |
  |---|---|---|---|
  | Full | 12 | 12 | **9** |
  | Slim | 3 | 3 | 3 |

  Full loses `BLUETOOTH`, `BLUETOOTH_ADMIN` (`maxSdkVersion` 30) and
  `WRITE_EXTERNAL_STORAGE` (28) by API 31, and API 28 still requests storage because
  the bound is inclusive. Slim requests only `INTERNET`, `ACCESS_NETWORK_STATE` and
  `VIBRATE` on every level, and carries no Full-only permission.

  Falsifiable: `--self-test` injects `READ_CONTACTS` and both profiles reject it,
  naming the permission.

  **Two holes an independent review found, both closed.** The parser read only
  `<uses-permission>`, so a `<uses-permission-sdk-23>` declaration — effective on every
  device this library supports, since its floor is API 26 — would have passed both gates
  while looking like a different element. And the debug-variant argument rested on
  source sets alone: a `releaseImplementation` dependency brings its own manifest into
  the merge and could add a permission to the shipped Release manifest only, so the gate
  now also refuses any build-type-scoped dependency configuration, which forces the
  release manifest to be compared directly if one is ever added.
- [ ] 1.14 Package the HarmonyOS HAR reproducibly with the unified version.

