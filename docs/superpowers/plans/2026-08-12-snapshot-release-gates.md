# Snapshot Release Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that every published Android AAR contains the V8 startup snapshot its slice manifest claims, extend the release freshness gate to the worker snapshot it currently ignores, and make the OpenHarmony package declare its snapshot policy instead of omitting it.

**Architecture:** Three independent tightenings, each turning a claim into a checkable fact about a real artifact. Two need no new code at all -- `check-snapshot-freshness.sh` already accepts `--snapshot-kind worker` and `build-ohos-sdk.sh` already writes a package manifest. The third adds one contract script that reads the shipped `.so`. Nothing inside `engine/crates/runtime-v8/` is touched, because that crate's own source tree is inside the snapshot fingerprint (see Task 2's rationale).

**Tech Stack:** Rust build scripts (`build.rs`), Bash contract gates, GitHub Actions YAML, Python 3 (PyYAML) for workflow parsing.

**Source spec:** `docs/superpowers/specs/2026-08-12-release-artifact-standard-design.md` — Problem section bullets 4 and 5, decision D4, step-1 items 9, 10 and 11.

**Commits:** this project does not auto-commit. Run the commit steps only when the user asks for them.

---

## Why these three and why together

The three defects share one root cause: **a claim is recorded in one place and enforced in another, or not at all.**

- `release.yml:25-28` runs the freshness gate for `host/full` and `host/slim` only, because `check-snapshot-freshness.sh:18` defaults `SNAPSHOT_KIND=host`. `build-snapshot.yml:94-96` runs all three variants on every push. So a stale `SNAPSHOT-worker-full-*.bin` is caught on a branch push but not at tag time — the release gate is strictly weaker than the development gate.
- `generate-android-artifact-manifests.py:353-361` validates the host snapshot unconditionally and refuses to emit a slice manifest when it is missing or stale, and `test-android-release-manifest-gate.sh` proves a release AAR cannot skip manifest generation. So the *file* cannot be stale. But `runtime-v8/build.rs` makes its own independent embedding decision and fails **safe** — every failure path prints `cargo:warning=...; loading JS from source` and continues. The slice manifest can therefore record a snapshot the `.so` does not contain.
- `build-ohos-sdk.sh:282` writes a package manifest with no `snapshot_policy` field, while `gen-android-package-metadata.py:200` writes `"embedded"` and `gen-linux-package-metadata.py:44` writes `"none"`. A consumer cannot tell whether OHOS has no snapshot or whether the field was forgotten.

Windows is deliberately **not** in this plan. `build-windows-sdk.sh:280` creates `share/migo/` but writes no package manifest at all, so `package-sdk.sh:84-91` refuses a Windows prefix outright. Authoring that manifest is a precondition for the Windows CI job, not a snapshot change.

## File structure

| File | Responsibility | Change |
| --- | --- | --- |
| `scripts/test-android-snapshot-embedding-contract.sh` | asserts a shipped AAR embeds the snapshot bytes its slice manifests claim | create |
| `.github/workflows/release.yml` | tag-time gate graph | add the worker freshness variant; add the embedding contract after the AAR build |
| `scripts/build-ohos-sdk.sh` | stages the OHOS SDK prefix and writes its package manifest | add `snapshot_policy` and a `known_gaps` entry |
| `scripts/test-ohos-sdk-contract.sh` | executable contract over a staged OHOS prefix | add a check that `snapshot_policy` is declared and matches what `build.rs` actually does |

Only one new file, and it holds one property. It deliberately does not re-verify what
`verify-android-aar-manifests.py` already checks (that the AAR's slices match the package
index) -- two implementations of one rule is the defect this repository keeps finding, not
a design.

---

## Task 1: Gate the worker snapshot at release time

**Files:**
- Modify: `.github/workflows/release.yml:25-28`
- Verify with: `scripts/check-snapshot-freshness.sh`, `scripts/test-release-gate-parity-contract.sh`

- [ ] **Step 1: Prove the gate is missing by running what the release does not run**

```bash
cd /data/work/opensource/migo
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full
```

Expected: PASS today (the committed worker snapshots are fresh). This establishes the
command is valid and currently green — it is the assertion the release workflow omits,
not a broken check.

- [ ] **Step 2: Prove the omission is load-bearing by making the worker snapshot stale**

The `.bin` and `.manifest.json` files are owned by `root` but their directory is owned by
the current user, so a rename over the file works while an in-place write does not. Save
the original first.

```bash
cd /data/work/opensource/migo
cp engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin.manifest.json \
   /tmp/worker-manifest.orig.json
python3 - <<'PY'
import json, os, pathlib
p = pathlib.Path("engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin.manifest.json")
data = json.loads(p.read_text(encoding="utf-8"))
data["js_sources_sha256"] = "0" * 64
tmp = p.with_name(p.name + ".tmp")
tmp.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
os.replace(tmp, p)
print("perturbed js_sources_sha256")
PY
```

- [ ] **Step 3: Confirm the release gate as it stands does NOT catch it**

```bash
cd /data/work/opensource/migo
bash scripts/check-snapshot-freshness.sh --product-profile full; echo "host/full exit=$?"
bash scripts/check-snapshot-freshness.sh --product-profile slim; echo "host/slim exit=$?"
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full; echo "worker/full exit=$?"
```

Expected: `host/full exit=0` and `host/slim exit=0` — the two commands the release
workflow runs both pass on a perturbed worker snapshot. `worker/full` exits non-zero and
prints a `js` mismatch line plus
`-> regenerate: scripts/gen-snapshot.sh x86_64 --product-profile full --snapshot-kind worker`.

This is the defect, measured: the release gate is green while a committed snapshot is stale.

- [ ] **Step 4: Restore the manifest**

```bash
cd /data/work/opensource/migo
mv /tmp/worker-manifest.orig.json \
   engine/crates/runtime-v8/snapshots/SNAPSHOT-worker-full-x86_64.bin.manifest.json
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full
```

Expected: PASS. If this does not pass, stop — the restore failed and the working tree is
dirty in a way that will confuse every later step. `git status` must show no change under
`engine/crates/runtime-v8/snapshots/`.

- [ ] **Step 5: Add the worker variant to the release gate**

In `.github/workflows/release.yml`, replace the body of the "Check snapshot freshness"
step:

```yaml
      # Block the release if any committed profile/ABI snapshot identity is
      # stale. Regenerate all affected products (see snapshots/README.md) before
      # tagging. The worker variant is checked explicitly because
      # check-snapshot-freshness.sh defaults --snapshot-kind to host, so the two
      # profile calls below cover neither worker blob -- build-snapshot.yml's
      # push gate ran all three and this one ran two, making the release gate
      # strictly weaker than the development gate.
      - name: Check snapshot freshness
        run: |
          bash scripts/check-snapshot-freshness.sh --product-profile full
          bash scripts/check-snapshot-freshness.sh --product-profile slim
          bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full
```

- [ ] **Step 6: Confirm the gate-parity contract still passes**

`test-release-gate-parity-contract.sh` requires every `pr-ci.yml` quality-gate step to
exist in `release.yml` with the same `run` body, and permits release-only extra steps.
This edit changes a release-only step's body, so the contract must stay green without an
exemption entry.

```bash
cd /data/work/opensource/migo
bash scripts/test-release-gate-parity-contract.sh
```

Expected: PASS. If it fails naming this step, the step also exists in `pr-ci.yml` and both
copies must be updated identically — read the failure message before changing anything.

- [ ] **Step 7: Confirm the workflow still parses**

```bash
cd /data/work/opensource/migo
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('release.yml parses')"
bash scripts/test-release-asset-ordering-contract.sh
```

Expected: `release.yml parses`, then the asset-ordering contract PASSes.

- [ ] **Step 8: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "Gate the worker V8 snapshot at release time, not only on push"
```

---

## Task 2: Prove the AAR contains the snapshot its manifest claims

**Files:**
- Create: `scripts/test-android-snapshot-embedding-contract.sh`
- Modify: `.github/workflows/release.yml` (new step in `release-android`, after the AAR build)

### Why not `build.rs`

The obvious fix is to make `runtime-v8/build.rs` panic instead of warn for an Android
release build. That was implemented and then reverted, because it does not work:
`build_snapshot.rs`'s `collect_rust_files` calls
`collect_files_with_extension(dir, "rs")` on the runtime-v8 **crate directory**, so
`build.rs` is itself inside the snapshot fingerprint. Editing it invalidated all six
committed snapshots at once -- `check-snapshot-freshness.sh --product-profile full`
reported both arches stale with `runtime-v8 Rust/op sources changed` -- and
regenerating the aarch64 pair needs physical arm64 hardware, since
`scripts/gen-snapshot.sh` runs the generator on the target ABI and hosted CI has no
arm64 KVM.

A validator that cannot be changed without hardware will not be changed. The artifact
gate below sits outside the fingerprint and proves a strictly stronger property: not
that the build script intended to embed, but that the bytes are in the binary.

- [ ] **Step 1: Read the real field names out of an existing AAR**

Do not guess the manifest schema. Ground it in an artifact.

```bash
cd /data/work/opensource/migo
python3 - <<'PYEOF'
import json, zipfile
p = "platforms/android/dist/migo-full-release-arm64-v8a.aar"
with zipfile.ZipFile(p) as z:
    idx = json.loads(z.read("assets/migo/artifacts/package-index.json"))
    for s in idx["slices"]:
        print("manifest_path:", s["manifest_path"])
        sl = json.loads(z.read(s["manifest_path"]))
        print("snapshots:", json.dumps(sl["snapshots"], indent=2))
    print("so entries:", [n for n in z.namelist() if n.endswith(".so")])
PYEOF
```

Expected: each package-index slice carries `manifest_path` (e.g.
`assets/migo/artifacts/slices/arm64-v8a.json`); each slice manifest carries a
`snapshots` array whose records have `runtime_kind`, `product_profile`, `arch` and
`bytes_hash`; the libraries are at `jni/<abi>/libmigo.so`.

- [ ] **Step 2: Write the gate**

Create `scripts/test-android-snapshot-embedding-contract.sh`. It takes one or more AAR
paths, and for every slice of every AAR: derives the snapshot filename from the slice's
own fields exactly as `build.rs` does (`SNAPSHOT-{profile}-{arch}.bin` for `host`,
`SNAPSHOT-worker-{profile}-{arch}.bin` for `worker`), checks the local blob's sha256
equals the slice's `bytes_hash`, and asserts those bytes occur inside
`jni/<abi>/libmigo.so`.

Three failure modes must each produce their own actionable message: the blob is absent
from the tree, its hash disagrees with the slice (stale AAR), and the library does not
contain it. An empty `snapshots` array is a failure, and so is being handed AARs that
collectively declare no snapshots -- a gate over nothing must not report PASS.

```bash
chmod +x scripts/test-android-snapshot-embedding-contract.sh
```

- [ ] **Step 3: Confirm the gate discriminates, using a stale AAR**

`platforms/android/dist/` still holds AARs from before the snapshots were regenerated.
That is the exact hazard `release.yml`'s `rm -rf platforms/android/dist` exists to
prevent, and it is free negative evidence.

```bash
cd /data/work/opensource/migo
bash scripts/test-android-snapshot-embedding-contract.sh \
  platforms/android/dist/migo-full-release-arm64-v8a.aar; echo "exit=$?"
```

Expected: `exit=1`, reporting that `SNAPSHOT-full-aarch64.bin` in this tree hashes to
one value while the slice claims another, and to rebuild the AAR.

- [ ] **Step 4: Build a current x86_64 release library and AAR**

A cold release build links V8 and Skia and takes tens of minutes. Run it detached and do
not start any other cargo, Gradle or Skia build while it runs -- there is one target
directory, one Gradle lock and one Skia environment on this machine.

```bash
cd /data/work/opensource/migo
bash scripts/build-android-so.sh x86_64 release 2>&1 | tail -5
bash scripts/build-aar.sh --product-profile full release x86_64 2>&1 | tail -5
ls -lh platforms/android/dist/*.aar
```

Expected: a fresh AAR whose mtime is newer than
`engine/crates/runtime-v8/snapshots/SNAPSHOT-full-x86_64.bin`.

- [ ] **Step 5: Confirm the gate passes on the fresh AAR**

```bash
cd /data/work/opensource/migo
bash scripts/test-android-snapshot-embedding-contract.sh \
  platforms/android/dist/migo-full-release.aar; echo "exit=$?"
```

Expected: `exit=0` with
`OK: ... slice x86_64: jni/x86_64/libmigo.so embeds SNAPSHOT-full-x86_64.bin (2158052 bytes)`.

If this reports that the library does not contain the blob, stop and check the build log
for `loading JS from source`: either the snapshot genuinely was not embedded, or
`include_bytes!` output is not byte-identical in the linked library, and the second case
means this gate's premise is wrong and needs a different observable.

- [ ] **Step 6: Mutation evidence -- corrupt the embedded copy and watch the gate fail**

Rewrite one copy of the AAR with the snapshot bytes inside its library altered. This
proves the gate observes the library, not just the manifest.

```bash
cd /data/work/opensource/migo
python3 - <<'PYEOF'
import json, pathlib, shutil, zipfile
src = pathlib.Path("platforms/android/dist/migo-full-release.aar")
dst = pathlib.Path("/tmp/migo-mutated.aar")
with zipfile.ZipFile(src) as z:
    idx = json.loads(z.read("assets/migo/artifacts/package-index.json"))
    abi = pathlib.PurePosixPath(idx["slices"][0]["manifest_path"]).stem
    entry = f"jni/{abi}/libmigo.so"
    blob = pathlib.Path("engine/crates/runtime-v8/snapshots/SNAPSHOT-full-x86_64.bin").read_bytes()
    items = {n: z.read(n) for n in z.namelist() if not n.endswith("/")}
offset = items[entry].find(blob)
assert offset >= 0, "snapshot bytes not found in the pristine library; Step 5 should have caught this"
library = bytearray(items[entry])
library[offset + 64] ^= 0xFF
items[entry] = bytes(library)
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as out:
    for name, payload in items.items():
        out.writestr(name, payload)
print(f"mutated one byte at library offset {offset + 64} -> {dst}")
PYEOF
bash scripts/test-android-snapshot-embedding-contract.sh /tmp/migo-mutated.aar; echo "exit=$?"
```

Expected: `exit=1`, reporting that `jni/x86_64/libmigo.so` does not contain the 2158052
bytes of `SNAPSHOT-full-x86_64.bin`. A single flipped byte is enough, which is the point:
the check is on the bytes, not on a length or a filename.

- [ ] **Step 7: Wire the gate into the release workflow**

In `.github/workflows/release.yml`, add a step immediately after
`Android minimum-API symbol floor` (which is the first step that runs once the AARs
exist):

```yaml
      # The slice manifests record which snapshot each ABI embeds, but nothing until
      # here reads the .so. build.rs decides embedding independently and fails safe --
      # every failure path warns and continues -- so a slice can claim a snapshot the
      # binary does not contain, costing cold-start time with nothing red. Named
      # explicitly rather than globbed: a glob that matches zero AARs would pass.
      - name: Android snapshot embedding contract
        run: |
          bash scripts/test-android-snapshot-embedding-contract.sh \
            platforms/android/dist/migo-full-release.aar \
            platforms/android/dist/migo-full-release-arm64-v8a.aar \
            platforms/android/dist/migo-slim-release.aar \
            platforms/android/dist/migo-slim-release-arm64-v8a.aar
```

The AAR list here is the pre-rename set. The canonical-naming plan replaces it, and the
naming contract added there is what keeps the two in step.

- [ ] **Step 8: Confirm the workflow parses and the release contracts still pass**

```bash
cd /data/work/opensource/migo
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('release.yml parses')"
bash scripts/test-release-gate-parity-contract.sh
bash scripts/test-release-asset-ordering-contract.sh
```

Expected: `release.yml parses` then two PASSes. The parity contract permits release-only
steps, so no exemption entry is needed.

- [ ] **Step 9: Confirm the committed snapshots were not disturbed**

```bash
cd /data/work/opensource/migo
git status --short engine/crates/runtime-v8/snapshots/
bash scripts/check-snapshot-freshness.sh --product-profile full
```

Expected: no output from `git status`, then PASS. This is the property the `build.rs`
approach could not hold.

- [ ] **Step 10: Commit**

```bash
git add scripts/test-android-snapshot-embedding-contract.sh .github/workflows/release.yml
git commit -m "Check that each AAR slice embeds the snapshot bytes its manifest claims"
```

---

## Task 3: Make the OpenHarmony package declare its snapshot policy

**Files:**
- Modify: `scripts/build-ohos-sdk.sh:282-305`
- Modify: `scripts/test-ohos-sdk-contract.sh`

The Android package manifest declares `snapshot_policy: "embedded"`
(`gen-android-package-metadata.py:200`), Linux declares `"none"`
(`gen-linux-package-metadata.py:44`), and OHOS declares nothing. Since
`runtime-v8/build.rs` embeds only when `target_os == "android"`, the truthful OHOS value
is `"none"`, and that must be checkable rather than assumed.

- [ ] **Step 1: Write the failing contract check**

In `scripts/test-ohos-sdk-contract.sh`, after the manifest is located (line 49-51) and
before the staged-bytes section (line 69), insert:

```bash
# ---- 1b. the manifest states its snapshot policy ----------------------------
# runtime-v8/build.rs embeds a V8 startup snapshot only when target_os ==
# "android", so an OHOS archive genuinely has none and its consumers pay a
# from-source cold start. The value is pinned rather than merely present-checked:
# if a future change teaches build.rs to embed for OpenHarmony, this check fails
# and forces the manifest to be updated in the same commit instead of silently
# under-claiming what the archive contains.
DECLARED_POLICY="$(python3 -c "
import json, sys
print(json.load(open(sys.argv[1])).get('snapshot_policy', '<missing>'))
" "$MANIFEST")"
if [[ "$DECLARED_POLICY" != "none" ]]; then
    fail "manifest snapshot_policy is '$DECLARED_POLICY'; expected 'none' (build.rs embeds for android only)"
    exit 1
fi
pass "manifest declares snapshot_policy=none, matching build.rs's android-only embedding"
```

- [ ] **Step 2: Run the contract against the existing staged prefix to watch it fail**

```bash
cd /data/work/opensource/migo
ls -d dist/migo-ohos-x86_64 2>/dev/null || bash scripts/build-ohos-sdk.sh x86_64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-x86_64 2>&1 | tail -15
```

Expected: FAIL with
`manifest snapshot_policy is '<missing>'; expected 'none' (build.rs embeds for android only)`.

- [ ] **Step 3: Add the field to the manifest heredoc**

In `scripts/build-ohos-sdk.sh`, inside the `cat > "$PREFIX/share/migo/ohos-$ARCH-manifest.json"`
heredoc at line 282, add one line directly after `"product_profile": "full",`:

```
  "snapshot_policy": "none",
```

- [ ] **Step 4: Record the consequence as a known gap**

In the same heredoc, add a first entry to the `known_gaps` array, immediately after
`"known_gaps": [$SURFACE_GAP`:

```
    "v8 startup snapshot: not embedded (build.rs embeds for android targets only), so first-run JS bootstrap parses extension sources instead of deserialising a heap",
```

- [ ] **Step 5: Rebuild the prefix and confirm the contract passes**

```bash
cd /data/work/opensource/migo
bash scripts/build-ohos-sdk.sh x86_64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-x86_64 2>&1 | tail -15
```

Expected: all checks PASS, including
`manifest declares snapshot_policy=none, matching build.rs's android-only embedding`.

- [ ] **Step 6: Confirm the manifest is still valid JSON and the packager accepts it**

The manifest is written by heredoc, so a stray comma is a real risk and
`package-sdk.sh` attests against this file.

```bash
cd /data/work/opensource/migo
python3 -m json.tool dist/migo-ohos-x86_64/share/migo/ohos-x86_64-manifest.json > /dev/null \
  && echo "manifest JSON valid"
bash scripts/package-sdk.sh dist/migo-ohos-x86_64 --output-dir /tmp/ohos-pkg-check 2>&1 | tail -5
```

Expected: `manifest JSON valid`, then `package-sdk.sh` produces a tarball and its
attestation without error.

- [ ] **Step 7: Confirm the aarch64 arch is equally covered**

The manifest is written once per arch from the same heredoc, so both arches must be
checked — the published release currently ships only OHOS x86_64, and aarch64 is the arch
real devices use.

```bash
cd /data/work/opensource/migo
bash scripts/build-ohos-sdk.sh aarch64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-aarch64 2>&1 | tail -15
```

Expected: all checks PASS.

- [ ] **Step 8: Commit**

```bash
git add scripts/build-ohos-sdk.sh scripts/test-ohos-sdk-contract.sh
git commit -m "Declare and check the OpenHarmony package's V8 snapshot policy"
```

---

## Full gate before handing off

Run once after all three tasks, not between them.

- [ ] **Step 1: Snapshot gates in the shape the release will run them**

```bash
cd /data/work/opensource/migo
bash scripts/check-snapshot-freshness.sh --product-profile full
bash scripts/check-snapshot-freshness.sh --product-profile slim
bash scripts/check-snapshot-freshness.sh --snapshot-kind worker --product-profile full
```

Expected: three PASSes.

- [ ] **Step 2: Workflow contracts**

```bash
cd /data/work/opensource/migo
bash scripts/test-release-gate-parity-contract.sh
bash scripts/test-release-asset-ordering-contract.sh
bash scripts/test-release-version-contract.sh
```

Expected: three PASSes.

- [ ] **Step 3: Package contracts for the platform whose manifest changed**

```bash
cd /data/work/opensource/migo
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-x86_64
bash scripts/test-ohos-sdk-contract.sh dist/migo-ohos-aarch64
bash scripts/package-sdk.sh dist/migo-ohos-x86_64 --output-dir /tmp/ohos-pkg-check
```

Expected: two PASSes then a tarball plus attestation. `package-sdk.sh` is included
because it attests against the manifest this task edited, so a heredoc comma error would
surface here even if the contract's own JSON read succeeded.

Android and Linux SDK contracts are deliberately **not** run. They would be required if
`engine/crates/runtime-v8/build.rs` had changed, since that file is shared by every
platform's artifact -- but the artifact-gate approach leaves `engine/` untouched. Confirm
that rather than assuming it:

```bash
cd /data/work/opensource/migo
git status --short engine/
```

Expected: no output. If anything is listed, the Android, Linux and host suites all become
mandatory again.

- [ ] **Step 4: Engine tests, only if `engine/` changed**

Skip when Step 3's `git status --short engine/` was empty -- no Rust source moved, so
there is nothing for the host suite to observe. Run it otherwise:

```bash
cd /data/work/opensource/migo/engine
cargo test 2>&1 | tail -30
```

Expected: no failures.

- [ ] **Step 5: Working tree contains only intended changes**

```bash
cd /data/work/opensource/migo
git status --short
```

Expected: only `engine/crates/runtime-v8/build.rs`, `.github/workflows/release.yml`,
`scripts/build-ohos-sdk.sh` and `scripts/test-ohos-sdk-contract.sh` if the commits were
not yet made, and nothing at all under `engine/crates/runtime-v8/snapshots/`. A modified
`.bin` or `.manifest.json` means a mutation step's restore failed.

---

## Follow-on plans

Three plans come out of the same spec. They are referred to by name, not number, because
one of them depends on another and ordinal references were ambiguous:

- **snapshot gates** -- this plan.
- **asset naming** -- `migo-<version>[-capi]-<platform>[-<arch>].<ext>` across
  `build-aar.sh` and `package-sdk.sh`; staged prefix directories normalised to
  `arm64`/`x86_64`; the slim AARs, the arm64-only AAR, `size-report-*.txt` and
  `version-{full,slim}.json` dropped; `scripts/test-release-asset-naming-contract.sh`
  added; every document that names an asset updated.
- **single publisher** -- publishing split out of `release-android` into a `publish` job
  that `needs` every platform job; `release-linux` added; `SHA256SUMS.txt` generated over
  the merged `dist/release/`; `scripts/verify-release-assets.sh` added;
  `scripts/publish-release.sh` deleted. Depends on **asset naming** for the canonical
  names its staging directory and naming contract assume.

`release-windows` and `release-ohos` are blocked on artifacts only the maintainer can
publish; see the spec's Blockers section.
