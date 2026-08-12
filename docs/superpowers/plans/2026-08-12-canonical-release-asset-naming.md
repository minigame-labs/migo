# Canonical Release Asset Naming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every published release asset one name shape — `migo-<version>[-capi]-<platform>[-<arch>].<ext>` — publish a single universal Android AAR instead of four variants, and drop the six metadata assets that carry no consumer value.

**Architecture:** Naming stays owned by the two scripts that already own it: `package-sdk.sh` derives tarball names from the staged prefix directory (its own comment: "Deriving it means there is no second naming scheme") and `build-aar.sh` composes the AAR name. Both need the version, which is read from `release/VERSION` — currently by four separate implementations of the same function, so the first task collapses those into one before adding a fifth consumer. A new contract gate then holds the shape.

**Tech Stack:** Bash build scripts, Python 3 contract gates, GitHub Actions YAML.

**Source spec:** `docs/superpowers/specs/2026-08-12-release-artifact-standard-design.md` — decisions D1, D2, and step-1 items 1-4 and 6.

**Commits:** this project does not auto-commit. Run the commit steps only when the user asks.

**Deviation from the writing-plans default:** the mechanical edits below are anchored by exact `file:line` and by the literal text being replaced, rather than reproducing every surrounding line. The non-obvious content — the naming rule, the shared reader, the contract, and what must NOT be renamed — is given in full.

---

## Target published set

```
migo-0.9.1-android.aar                      34 MB   arm64-v8a + x86_64
migo-0.9.1-capi-android-arm64.tar.gz        71 MB
migo-0.9.1-capi-android-x86_64.tar.gz       69 MB
migo-0.9.1-capi-linux-x86_64.tar.gz         72 MB
migo-0.9.1-capi-ohos-arm64.tar.gz          ~88 MB
migo-0.9.1-capi-ohos-x86_64.tar.gz          88 MB
migo-0.9.1-capi-windows-x86_64.tar.gz       27 MB
migo-0.9.1-sbom.cdx.json
version.json
+ one .attestation.json per payload
```

Gone: `migo-slim-release.aar`, `migo-slim-release-arm64-v8a.aar`, `migo-full-release-arm64-v8a.aar`, `size-report-full.txt`, `size-report-slim.txt`, `version-full.json`, `version-slim.json`.

**`SHA256SUMS.txt` stays — corrected during execution.** The plan and the spec both said
to delete it because it covers only the Android job's output while its name implies the
whole release. Retargeting it at `dist/release/` removes the dishonesty at the source: the
staging directory holds exactly what this job publishes, so `sha256sum *` over it
over-claims nothing. The lie was in the *input range*, not the file, and deleting a
mechanism that had merely been pointed at the wrong directory would have been a loss.
`scripts/test-release-asset-ordering-contract.sh` also fails closed when no step writes
the manifest, which is what surfaced this.

## What must NOT be renamed

- **`CHANGELOG.md` v0.9.1 and earlier entries.** Those releases shipped under the old names; rewriting them falsifies a historical record. New names appear in the next version's entry.
- **`docs/superpowers/` specs and plans dated before today.** Same reason — they record what was true when written.
- **`jni/arm64-v8a/` inside the AAR.** Android mandates that directory name.
- **Rust target triples** (`aarch64-linux-android`, `x86_64-linux-ohos`) anywhere in build scripts. `arm64` is the public vocabulary; the triple is the toolchain's.

## File structure

| File | Change |
| --- | --- |
| `scripts/lib/release-version.sh` | create — the one `read_release_version` |
| `scripts/build-{linux,ohos,windows}-sdk.sh` | source the lib, delete the local copy |
| `scripts/build-android-sdk.sh` | source the lib, delete the inline copy; stage `dist/migo-android-<canonical arch>` |
| `scripts/build-ohos-sdk.sh` | stage `dist/migo-ohos-<canonical arch>` |
| `scripts/package-sdk.sh` | derive `migo-<version>-capi-<platform>-<arch>.tar.gz` |
| `scripts/build-aar.sh` | emit `migo-<version>-android.aar` for the publishable configuration |
| `scripts/test-release-version-contract.sh` | one definition site; add `package-sdk.sh` as a consumer |
| `scripts/test-release-asset-naming-contract.sh` | create |
| `scripts/test-android-snapshot-embedding-contract.sh` | its release call site loses three AARs |
| `.github/workflows/release.yml` | build one AAR; drop six metadata assets; run the naming contract |
| `scripts/test-{android,ohos}-sdk-contract.sh` | follow the renamed staged prefixes |
| `BUILD.md`, `platforms/android/README.md`, `platforms/android/README_EN.md` | document the new names |

---

## Task 1: One reader for `release/VERSION`

Four implementations of one rule exist today: identical `read_release_version()` in
`build-linux-sdk.sh:145`, `build-ohos-sdk.sh:82`, `build-windows-sdk.sh:92`, and an
inline copy in `build-android-sdk.sh:36-41`. `package-sdk.sh` needs the version too, and
adding a fifth is the wrong move — this repository's recurring defect is two
implementations of one rule.

- [ ] **Step 1: Create `scripts/lib/release-version.sh`**

```bash
#!/usr/bin/env bash
# The one reader of release/VERSION, which
# scripts/test-release-version-contract.sh holds as the single version source.
#
# This existed four times over -- identically in build-linux-sdk.sh,
# build-ohos-sdk.sh and build-windows-sdk.sh, and inline in build-android-sdk.sh --
# and the contract had to enumerate each copy's assignment literal to check them.
# package-sdk.sh became the fifth consumer, which is what forced the collapse.
#
# Intended to be sourced, not executed.

read_release_version() {
    local source="$1/release/VERSION"
    [[ -f "$source" ]] || { echo "[release-version] source missing: $source" >&2; exit 1; }
    local version
    version="$(tr -d '[:space:]' < "$source")"
    [[ -n "$version" ]] || { echo "[release-version] source is empty: $source" >&2; exit 1; }
    printf '%s' "$version"
}
```

- [ ] **Step 2: Replace the three copies with a source line**

In each of `build-linux-sdk.sh`, `build-ohos-sdk.sh`, `build-windows-sdk.sh`, delete the
8-line `read_release_version() { ... }` block and put this immediately above the
assignment that follows it (each script already computes `$REPO_ROOT` and sources other
`scripts/lib/*.sh` helpers the same way):

```bash
# shellcheck source=scripts/lib/release-version.sh
source "$REPO_ROOT/scripts/lib/release-version.sh"
```

Leave the assignment lines exactly as they are — `VERSION="$(read_release_version
"$REPO_ROOT")"` in linux and windows, `MIGO_VERSION="$(...)"` in ohos — because
`test-release-version-contract.sh`'s `DERIVES` table matches on those literals.

- [ ] **Step 3: Replace `build-android-sdk.sh`'s inline copy**

Replace lines 35-41 (the `if [[ -z "$VERSION" ]]; then ... fi` block that reads
`$REPO_ROOT/release/VERSION` directly) with:

```bash
# shellcheck source=scripts/lib/release-version.sh
source "$REPO_ROOT/scripts/lib/release-version.sh"
if [[ -z "$VERSION" ]]; then
    VERSION="$(read_release_version "$REPO_ROOT")"
fi
```

The `-z` guard stays: an explicit `--version` argument must still win.

- [ ] **Step 4: Point the contract at the one definition and guard against a second**

In `scripts/test-release-version-contract.sh`, change the `build-android-sdk.sh` entry of
`DERIVES` from `'SOURCE="$REPO_ROOT/release/VERSION"'` to
`'VERSION="$(read_release_version "$REPO_ROOT")"'`, and add a check that only the library
defines the function — the consolidation is worth nothing if a copy can reappear:

```python
# ------------------------------------------------ one definition of the reader

# The four build scripts each carried an identical read_release_version(); the
# contract could only check that each *called* something, not that they called the
# same thing. A reintroduced copy would pass every check above, so the definition
# site is pinned here.
DEFINITION = "scripts/lib/release-version.sh"
definers = sorted(
    str(candidate.relative_to(root))
    for candidate in root.glob("scripts/**/*.sh")
    if "read_release_version()" in candidate.read_text(encoding="utf-8")
)
if definers != [DEFINITION]:
    errors.append(
        f"read_release_version() must be defined only in {DEFINITION}, but is defined in "
        f"{definers}. One rule, one implementation: a second copy can drift from "
        "release/VERSION while every call site still looks correct"
    )
```

Place it immediately before the `INDEPENDENT` section so it runs inside the same `errors`
accumulation.

- [ ] **Step 5: Verify the contract passes and still counts five consumers**

```bash
cd /data/work/opensource/migo
bash scripts/test-release-version-contract.sh
```

Expected: PASS, reporting `release/VERSION = 0.9.1` and `5 build consumers derive from it`.

- [ ] **Step 6: Verify the guard actually fires (mutation evidence)**

```bash
cd /data/work/opensource/migo
cp scripts/build-linux-sdk.sh /tmp/build-linux-sdk.sh.orig
cat >> scripts/build-linux-sdk.sh <<'EOF'

read_release_version() { printf '%s' "9.9.9"; }
EOF
bash scripts/test-release-version-contract.sh 2>&1 | tail -4; echo "exit=${PIPESTATUS[0]}"
mv /tmp/build-linux-sdk.sh.orig scripts/build-linux-sdk.sh
bash scripts/test-release-version-contract.sh 2>&1 | tail -1
```

Expected: the middle run exits non-zero naming both `scripts/build-linux-sdk.sh` and the
library as definers; the final run PASSes and `git status --short scripts/` is clean.

- [ ] **Step 7: Verify each build script still resolves the version**

The contract checks text, not behaviour. Prove the sourced helper actually runs by
rebuilding the one SDK that is cheap here and reading the version out of its staged
output.

```bash
cd /data/work/opensource/migo
bash scripts/build-ohos-sdk.sh x86_64 2>&1 | tail -2
grep -m1 MIGO_VERSION dist/migo-ohos-x86_64/lib/cmake/migo/migo-config.cmake
```

Expected: `set(MIGO_VERSION "0.9.1")`.

- [ ] **Step 8: Commit**

```bash
git add scripts/lib/release-version.sh scripts/build-linux-sdk.sh scripts/build-ohos-sdk.sh \
        scripts/build-windows-sdk.sh scripts/build-android-sdk.sh \
        scripts/test-release-version-contract.sh
git commit -m "Collapse four copies of the release-version reader into one library"
```

---

## Task 2: Canonical arch vocabulary in staged prefixes

`package-sdk.sh` derives the asset name from the staged prefix directory name, so the
vocabulary has to be fixed at the source rather than translated in the packager — a
translation table would be the second vocabulary the derivation exists to avoid.

Today: `dist/migo-android-arm64-v8a`, `dist/migo-android-x86_64`,
`dist/migo-ohos-aarch64`, `dist/migo-ohos-x86_64`, `dist/migo-linux-x86_64`,
`dist/migo-windows-x86_64`. Target: `arm64` and `x86_64` everywhere.

- [ ] **Step 1: Find every producer and consumer of the staged prefix names**

```bash
cd /data/work/opensource/migo
grep -rn "migo-android-\$\|migo-android-arm64-v8a\|migo-ohos-\$\|dist/migo-" \
  scripts/ .github/workflows/ | grep -v "^scripts/test-release-asset"
```

Expected: the two build scripts that construct the path, the two SDK contracts that read
it, and the `release.yml` loops that pass an arch. Record the list before editing; each
one must change together or the packager will not find its prefix.

- [ ] **Step 2: Make `build-android-sdk.sh` stage the canonical arch**

`build-android-sdk.sh:43-45` maps `--arch aarch64` to `ABI=arm64-v8a` and uses the ABI in
the staged prefix. Keep `ABI` for the Android toolchain and jniLibs layout, where
`arm64-v8a` is mandatory, and introduce a separate public word for the path:

```bash
case "$ARCH" in
    aarch64) TARGET="aarch64-linux-android"; ABI="arm64-v8a"; PUBLIC_ARCH="arm64" ;;
    x86_64)  TARGET="x86_64-linux-android";  ABI="x86_64";    PUBLIC_ARCH="x86_64" ;;
```

Then use `PUBLIC_ARCH` in the staged prefix path only. Do not substitute it anywhere the
NDK, cargo-ndk or Gradle reads an ABI name.

- [ ] **Step 3: Make `build-ohos-sdk.sh` stage the canonical arch**

`build-ohos-sdk.sh` uses `$ARCH` (`aarch64`/`x86_64`) for the triple, the staged prefix
and the in-package manifest filename. Add `PUBLIC_ARCH` (`aarch64` -> `arm64`) and use it
for **the prefix directory only** — three sites: the `PREFIX=` assignment and the two
later references that re-derive the path for the contract call and the symbol-floor input.

Corrected during execution: an earlier version of this step also renamed
`ohos-<arch>-manifest.json`. That is wrong on two counts. The manifest lives *inside* the
package, so it is not a published asset name and the public vocabulary does not govern it;
and `test-ohos-sdk-contract.sh:51` derives the arch from that filename to decide which
musl loader to expect (`/lib/ld-musl-aarch64.so.1`), so renaming it breaks a check for no
benefit. Same principle keeps `jni/arm64-v8a/` inside the AAR and
`android-arm64-v8a-manifest.json` inside the Android SDK: public names use the public
word, internal paths use the toolchain's.

- [ ] **Step 4: Follow the rename in both SDK contracts**

`test-ohos-sdk-contract.sh:49-51` derives `ARCH` from the manifest filename by stripping
`ohos-` and `-manifest.json`, so it needs no change once the filename is canonical — but
its `musl loader` expectation is keyed on the arch word, so re-read it. Update
`test-android-sdk-contract.sh` wherever it constructs `dist/migo-android-<...>`.

- [ ] **Step 5: Verify both platforms end to end**

```bash
cd /data/work/opensource/migo
rm -rf dist/migo-ohos-aarch64 dist/migo-android-arm64-v8a
bash scripts/build-ohos-sdk.sh aarch64 2>&1 | tail -2
ls -d dist/migo-ohos-arm64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-arm64 2>&1 | grep -cE "PASS"
bash scripts/build-android-sdk.sh --arch aarch64 2>&1 | tail -2
ls -d dist/migo-android-arm64
bash scripts/test-android-sdk-contract.sh --arch aarch64 2>&1 | tail -3
```

Expected: both canonical prefixes exist, the OHOS contract reports 7 passes, and the
Android contract passes. A leftover `dist/migo-ohos-aarch64` means Step 3 missed a path.

- [ ] **Step 6: Commit**

```bash
git add scripts/build-android-sdk.sh scripts/build-ohos-sdk.sh \
        scripts/test-android-sdk-contract.sh scripts/test-ohos-sdk-contract.sh
git commit -m "Stage SDK prefixes under one public architecture vocabulary"
```

---

## Task 3: Canonical tarball names

- [ ] **Step 1: Change the derivation in `package-sdk.sh`**

Replace `package-sdk.sh:79`, `ASSET="migo-sdk-${PREFIX_NAME#migo-}.tar.gz"`, with a
version-bearing form. Source the library from Task 1 near the top of the script and derive:

```bash
# shellcheck source=scripts/lib/release-version.sh
source "$ROOT/scripts/lib/release-version.sh"
VERSION="$(read_release_version "$ROOT")"
# migo-<os>-<arch> (the staged prefix) -> migo-<version>-capi-<os>-<arch>.tar.gz.
# Still derived from the prefix rather than assembled from separate arguments, for the
# reason the original comment gives: a second naming scheme is a second thing to keep
# in step. `capi` is what distinguishes these from the .aar, whose extension already
# says "Android, Java/Kotlin".
ASSET="migo-${VERSION}-capi-${PREFIX_NAME#migo-}.tar.gz"
```

Keep the existing `migo-*` prefix guard at lines 73-77 unchanged.

- [ ] **Step 2: Register the new consumer with the version contract**

Add to `DERIVES` in `scripts/test-release-version-contract.sh`:

```python
    "scripts/package-sdk.sh": (
        'VERSION="$(read_release_version "$ROOT")"',
        "every C-ABI SDK tarball's published filename",
    ),
```

- [ ] **Step 3: Verify the name and that the attestation agrees with it**

The attestation records `package_file`, so a rename that misses it produces a package
whose sidecar names a file nobody received.

```bash
cd /data/work/opensource/migo
rm -rf /tmp/pkg-name-check
bash scripts/package-sdk.sh dist/migo-ohos-arm64 --output-dir /tmp/pkg-name-check 2>&1 | tail -3
ls /tmp/pkg-name-check
python3 -c "
import json; print(json.load(open('/tmp/pkg-name-check/migo-0.9.1-capi-ohos-arm64.tar.gz.attestation.json'))['package_file'])"
```

Expected: `migo-0.9.1-capi-ohos-arm64.tar.gz` plus its sidecar, and the sidecar's
`package_file` equals that name.

- [ ] **Step 4: Confirm reproducibility still holds**

```bash
cd /data/work/opensource/migo
bash scripts/test-sdk-package-reproducibility-contract.sh 2>&1 | tail -4
bash scripts/test-release-version-contract.sh 2>&1 | tail -1
```

Expected: both PASS, the version contract now reporting `6 build consumers`.

- [ ] **Step 5: Commit**

```bash
git add scripts/package-sdk.sh scripts/test-release-version-contract.sh
git commit -m "Publish C-ABI SDK tarballs under a version-bearing canonical name"
```

---

## Task 4: One universal AAR under the canonical name

`build-aar.sh:551` composes
`migo-$PRODUCT_PROFILE-$BUILD_TYPE$ARTIFACT_SUFFIX$ABI_ARTIFACT_SUFFIX.aar`, where
`ARTIFACT_SUFFIX` carries a non-default codegen profile and the worker-snapshot opt-in,
and `ABI_ARTIFACT_SUFFIX` marks a single-ABI build. Exactly one combination is
publishable: `full`, `release`, both suffixes empty. That combination gets the canonical
name; every other keeps its descriptive one, so two variants can still never overwrite
each other in `dist/`.

The single-ABI variant stays *buildable* — its comment explains it exists because per-ABI
size is a procurement question, and measuring that is still useful. It just stops being
published; spec decision D2 answers the size question with documentation instead.

- [ ] **Step 1: Add the canonical name for the publishable configuration**

At `build-aar.sh:551`, replace the single `artifact_name` assignment with:

```bash
    # The published artifact has one name; every internal variant keeps a descriptive
    # one so it cannot overwrite the published file in dist/. Gradle's own
    # <profile><buildType> name is an internal detail and stops here -- it was
    # reaching consumers as migo-full-release.aar, which told them about a product
    # profile they cannot choose.
    local artifact_name
    if [[ "$PRODUCT_PROFILE" == "full" && "$BUILD_TYPE" == "release" \
          && -z "$ARTIFACT_SUFFIX" && -z "$ABI_ARTIFACT_SUFFIX" ]]; then
        artifact_name="migo-$(read_release_version "$REPO_ROOT")-android.aar"
    else
        artifact_name="migo-$PRODUCT_PROFILE-$BUILD_TYPE$ARTIFACT_SUFFIX$ABI_ARTIFACT_SUFFIX.aar"
    fi
```

Source `scripts/lib/release-version.sh` near the top of `build-aar.sh` alongside its other
`scripts/lib` sources, and add a `DERIVES` entry for it:

```python
    "scripts/build-aar.sh": (
        'artifact_name="migo-$(read_release_version "$REPO_ROOT")-android.aar"',
        "the published Android AAR's filename",
    ),
```

- [ ] **Step 2: Verify both branches of the rule**

```bash
cd /data/work/opensource/migo
rm -f platforms/android/dist/*.aar platforms/android/dist/*.attestation.json
bash scripts/build-aar.sh --product-profile full release 2>&1 | tail -3
bash scripts/build-aar.sh --product-profile full release x86_64 2>&1 | tail -3
ls platforms/android/dist/*.aar
```

Expected exactly two files: `migo-0.9.1-android.aar` (the publishable, both ABIs) and
`migo-full-release-x86_64.aar` (single-ABI, internal). If the second one is also named
`migo-0.9.1-android.aar` the guard is wrong and the two builds overwrote each other.

- [ ] **Step 3: Confirm the AAR still contains both ABIs**

The whole argument for one AAR is that the consumer's `abiFilters` prunes it, which only
holds if both ABIs are in there.

```bash
cd /data/work/opensource/migo
python3 -c "
import zipfile
with zipfile.ZipFile('platforms/android/dist/migo-0.9.1-android.aar') as z:
    print(sorted(n for n in z.namelist() if n.endswith('libmigo.so')))"
```

Expected: `['jni/arm64-v8a/libmigo.so', 'jni/x86_64/libmigo.so']`.

- [ ] **Step 4: Confirm the snapshot embedding contract follows the rename**

```bash
cd /data/work/opensource/migo
bash scripts/test-android-snapshot-embedding-contract.sh \
  platforms/android/dist/migo-0.9.1-android.aar; echo "exit=$?"
```

Expected: `exit=0` with two OK lines, one per ABI slice.

- [ ] **Step 5: Commit**

```bash
git add scripts/build-aar.sh scripts/test-release-version-contract.sh
git commit -m "Publish a single universal Android AAR under a version-bearing name"
```

---

## Task 5: The naming contract

- [ ] **Step 1: Create `scripts/test-release-asset-naming-contract.sh`**

Takes a directory. Every file in it must match
`migo-<version>(-capi)?-<platform>(-<arch>)?\.(aar|tar\.gz)`, or be
`migo-<version>-sbom.cdx.json`, or be a `<matching asset>.attestation.json`, or be one of
two named clerical files. The version segment must equal `release/VERSION`. Platforms are
`android|linux|windows|ohos`; architectures are `arm64|x86_64`. An empty directory is a
failure.

The two exemptions are `version.json` and `SHA256SUMS.txt`, and the reason belongs in the
script: their URLs must stay predictable across versions, so a consumer can fetch
`.../releases/download/v<tag>/SHA256SUMS.txt` without first knowing the version. Every
other exemption request should be refused.

This subsumes `release.yml`'s `rm -rf platforms/android/dist` precaution: a stale asset
from an older naming scheme now fails a gate instead of being silently uploaded.

- [ ] **Step 2: Verify against the real staged directory**

```bash
cd /data/work/opensource/migo
mkdir -p /tmp/naming-check && rm -f /tmp/naming-check/*
cp platforms/android/dist/migo-0.9.1-android.aar* /tmp/naming-check/
cp /tmp/pkg-name-check/* /tmp/naming-check/
bash scripts/test-release-asset-naming-contract.sh /tmp/naming-check; echo "exit=$?"
```

Expected: `exit=0`, listing each accepted name.

- [ ] **Step 3: Mutation evidence — plant an old-scheme name**

```bash
cd /data/work/opensource/migo
touch /tmp/naming-check/migo-full-release.aar
bash scripts/test-release-asset-naming-contract.sh /tmp/naming-check; echo "exit=$?"
rm /tmp/naming-check/migo-full-release.aar
touch /tmp/naming-check/migo-0.9.0-capi-linux-x86_64.tar.gz
bash scripts/test-release-asset-naming-contract.sh /tmp/naming-check; echo "exit=$?"
rm /tmp/naming-check/migo-0.9.0-capi-linux-x86_64.tar.gz
bash scripts/test-release-asset-naming-contract.sh /tmp/naming-check; echo "exit=$?"
```

Expected: `exit=1` for the old-scheme name, `exit=1` again for the right shape with the
wrong version, then `exit=0`. The second case matters most — a wrong version segment is
the failure a shape-only regex would miss.

- [ ] **Step 4: Commit**

```bash
git add scripts/test-release-asset-naming-contract.sh
git commit -m "Hold the published asset name shape with an executable contract"
```

---

## Task 6: Stage the published set, then trim what is not in it

**Forced design change, derived during execution.** Version-bearing asset names make a
literal asset list in `release.yml` impossible — YAML cannot interpolate
`release/VERSION`, and hardcoding `0.9.1` in the workflow would put a second version
source next to the one `test-release-version-contract.sh` exists to protect. So the
staging directory from spec decision D6 has to arrive here rather than in the **single
publisher** plan:

- a step computes `VERSION="$(cat release/VERSION)"`, recreates `dist/release/`, and
  copies each published asset into it from the path its producer wrote;
- `test-release-asset-naming-contract.sh dist/release` runs next;
- the publish step becomes `files: dist/release/*`.

That also answers what replaces the 18-name presence check, whose value was that a build
which silently stopped producing an artifact failed loudly. The staging step's `cp` is
that assertion: a missing source file makes `cp` fail, and the workflow's `bash -e` aborts
the job. The list of source paths exists once, in the step that knows them, instead of
three times.

`version-full.json` and `version-slim.json` therefore stay *written* — checked during
execution, nothing reads them, but they record build configuration (codegen profile, cargo
profile, worker snapshot, `SOURCE_DATE_EPOCH`) that the release-level `version.json` does
not. They simply stop being copied into `dist/release/`, which is what makes them internal
rather than deleted.


- [ ] **Step 1: Trim the AAR build step**

In `.github/workflows/release.yml`, the "Build Android AARs (release)" step builds four
AARs. Reduce it to the publishable one plus the SBOM, keeping the `rm -rf` and the SBOM
generation. Record in the step's comment why slim is no longer built here: the profile is
internal, and what guards it is `scripts/test-product-profiles.sh` in the quality gate,
not an uploaded artifact.

- [ ] **Step 2: Delete the size-report and split-version steps**

Remove the "Analyze AAR size report" step — it wrote `size-report-{full,slim}.txt` with
`|| true`, and per-symbol size attribution is internal. If it is wanted, it belongs in a
CI artifact, not a release asset. Remove `version-full.json` and `version-slim.json` from
the presence check and the upload list; keep the single `version.json`.

- [ ] **Step 3: Delete the `SHA256SUMS.txt` step and its upload entry**

It covers only this job's output while its name implies the release. The **single
publisher** plan reinstates it from a job that sees everything.
`test-release-asset-ordering-contract.sh` asserts that step is the sole writer, so removing
it must keep that contract green — read its failure message before assuming otherwise.

- [ ] **Step 4: Update the remaining asset names in the workflow**

The presence check, the `files:` list, the snapshot embedding contract call and the
`package-sdk.sh` loop all name assets. Point them at the canonical names and add a step
running `test-release-asset-naming-contract.sh` on `platforms/android/dist` immediately
before the publish step.

- [ ] **Step 5: Verify the workflow and every affected contract**

```bash
cd /data/work/opensource/migo
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('parses')"
bash scripts/test-release-asset-ordering-contract.sh
bash scripts/test-release-gate-parity-contract.sh
bash scripts/test-release-version-contract.sh
bash scripts/test-product-profiles.sh
```

Expected: `parses` then four PASSes. `test-product-profiles.sh` is the check that has to
carry slim's coverage now that no slim artifact is uploaded — if it does not actually
exercise the slim feature set, say so rather than treating slim as covered.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "Publish one Android AAR and drop the metadata assets consumers cannot use"
```

---

## Task 7: Documentation

Only three files describe assets to a consumer. The sweep that established this also
found `CHANGELOG.md` and the older `docs/superpowers/` documents, which are historical
records and must not be rewritten.

- [ ] **Step 1: Update the Android integration instructions**

`platforms/android/README.md:30` and `platforms/android/README_EN.md:30` tell an
integrator to drop `migo-full-release.aar` into a Gradle build. Change the filename, and
add the size note that replaces the arm64-only variant:

> The AAR carries both `arm64-v8a` and `x86_64` (34 MB). A shipped app carries one ABI:
> add `ndk { abiFilters 'arm64-v8a' }` to your `defaultConfig` and the packaged native
> library is 17 MB, or publish an App Bundle and Play delivers per device.

- [ ] **Step 2: Update `BUILD.md`**

`BUILD.md:465-473` shows `package-sdk.sh` output and explains that the asset name is
derived from the staged prefix. Update the example names and the derivation sentence, and
correct the `SHA256SUMS.txt` claim at line 473 — it describes coverage that is being
removed here and reinstated later.

- [ ] **Step 3: Verify no user-facing document still names a dropped asset**

```bash
cd /data/work/opensource/migo
grep -rn "migo-full-release\|migo-slim-release\|migo-sdk-android\|size-report\|version-full" \
  README.md README.zh-CN.md BUILD.md platforms/*/README*.md include/migo/README.md
```

Expected: no output. Hits under `CHANGELOG.md` or `docs/superpowers/` are correct and out
of scope.

- [ ] **Step 4: Commit**

```bash
git add BUILD.md platforms/android/README.md platforms/android/README_EN.md
git commit -m "Document the canonical asset names and the per-ABI size story"
```

---

## Full gate before handing off

```bash
cd /data/work/opensource/migo
bash scripts/test-release-version-contract.sh
bash scripts/test-release-asset-naming-contract.sh platforms/android/dist
bash scripts/test-release-asset-ordering-contract.sh
bash scripts/test-release-gate-parity-contract.sh
bash scripts/test-sdk-package-reproducibility-contract.sh
bash scripts/test-product-profiles.sh
bash scripts/test-android-snapshot-embedding-contract.sh platforms/android/dist/migo-0.9.1-android.aar
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-arm64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-x86_64
git status --short engine/
```

Expected: every contract PASSes and `git status --short engine/` is empty — this batch
touches no engine source, so the snapshot fingerprint is undisturbed.

## Next

The **single publisher** plan follows: publishing split out of `release-android` into a
`publish` job that `needs` every platform job, `release-linux` added, `SHA256SUMS.txt`
reinstated over the merged `dist/release/`, `scripts/verify-release-assets.sh` added, and
`scripts/publish-release.sh` deleted — which is what ends the current race where that
script pushes a tag, the workflow publishes, and the script then re-creates the same
release and `--clobber`s the assets.
