# Windows CI job (Phase 1 of platform-matrix unification)

## Problem

Every platform `v0.9.2` ships — Android, Linux, OpenHarmony — is built, packaged, and
published by `release.yml` with no human in the loop. Windows is the one exception:
`scripts/build-windows-sdk.sh` produces a correct, contract-verified package, but only
on a machine that is simultaneously WSL2 and a native Windows install, because the
script depends on `platforms/windows/spike/lib.sh`'s WSL↔Windows crossing —
`wslpath` DOS-path translation, a synced `/mnt/c/migo-win` worktree
(`require_synced_worktree`), and `cmd.exe`-dispatched batch files
(`run_windows_batch`). None of that exists on a GitHub-hosted `windows-latest`
runner: the checkout already lands on native NTFS, so there is no WSL/Windows
boundary for that machinery to cross. The result is that the Windows package for
every release to date has been built by hand and uploaded after the fact —
recorded honestly in the package's own manifest (`known_gaps`) and in
`verify-release-assets.sh`'s output, which lists it as covered by its own
attestation sidecar rather than the release-wide `SHA256SUMS.txt`.

This is the first of three planned phases toward full CI coverage
(android/linux/ohos/windows × x86_64/arm64, nothing built by hand). Phase 1 covers
Windows x86_64 only, because every dependency it needs already exists: the V8
archive is published (`v8-archives-e6a88b3`, target `x86_64-pc-windows-msvc`), the
ANGLE runtime is pinned and fetchable (`contracts/artifact-manifest/windows-angle.lock.json`,
`scripts/fetch-windows-angle.sh`), and the build/package/contract scripts already
produce a correct artifact — they just can't run in CI as written. Linux arm64 and
Windows arm64 (Phases 2 and 3) are net-new platforms with no existing V8 archive,
sysroot, or build script, and are out of scope here; they get their own specs.

## Goals

- A `release-windows` job in `release.yml` that builds, packages, and
  contract-verifies the Windows x86_64 SDK with no human step, running in parallel
  with `release-android`/`release-linux`/`release-ohos` and gating `publish` exactly
  like them — a red Windows build blocks the release the same way a red Android
  build already does.
- The published Windows asset becomes indistinguishable, in provenance, from every
  other platform's: covered by the release-wide `SHA256SUMS.txt`, not a hand-typed
  `tar` with a separately-argued attestation.
- The existing WSL-based local dev workflow (`build-windows-sdk.sh`) keeps working
  unmodified in its externally-visible behavior, for iterating on this machine
  without waiting on CI.

## Non-goals

- V8 startup snapshot embedding for Windows (`snapshot_policy` stays `"none"`,
  matching today — `runtime-v8/build.rs` does not embed for `windows` yet; that is
  separate work, not blocked by or blocking this).
- NuGet packaging (the manifest already records this as a known gap; unaffected).
- Any arm64 target, on either platform (Phases 2/3).
- Any change to what the *local* WSL developer workflow requires installed
  (Visual Studio Build Tools, LLVM, etc.) — it is untouched, not re-validated.

## Decisions

### D1 — Job placement: full parity from day one, no observation period

`release-windows` is added alongside the other three `release-*` jobs, depending on
`[quality-gate, host-engine-tests]` (matching `release-linux`'s dependency set — the
closest structural analog), and `publish`'s `needs` gains `release-windows`
alongside the existing three. No staged rollout where it runs non-blocking first:
the scripts it runs are already proven correct (they produce the package this
session hand-verified and published for `v0.9.2`), so the only new risk is CI
*environment* mechanics, not artifact correctness — exactly what needs to be caught
before this is trusted, not after.

### D2 — A new script, not a branch in the existing one

New file: `scripts/build-windows-sdk-native.sh`. It does **not** live inside
`build-windows-sdk.sh` behind an `if running-natively` branch. It defaults its
staging prefix to `$REPO_ROOT/dist/migo-windows-x86_64` — the same default
`build-windows-sdk.sh` already uses — so that `test-windows-sdk-contract.sh`'s own
default (`MIGO_WINDOWS_PREFIX:-$REPO_ROOT/dist/migo-windows-x86_64`, see D6) finds
the package without either script needing to pass the path explicitly.

`build-windows-sdk.sh`'s entire shape — locating `vcvars64.bat` via `vswhere`,
building a curated PATH allowlist that keeps a stray Android NDK from shadowing
`clang-cl`, generating and dispatching `.bat` files through `cmd.exe`, translating
every path through `wslpath` — exists solely to cross the WSL/Windows boundary.
On a native runner there is no boundary: `cargo build`, `link.exe`, and the fetch
scripts all just run, in the job's own working directory, with no translation
layer. Branching one file on "am I crossing a boundary or not" would interleave
two unrelated execution models in one control flow, where today each concern
(WSL-crossing vs. native execution) is legible on its own. Two callers is the
simpler shape.

### D3 — Shared library for the environment-independent tail

New file: `scripts/lib/windows-sdk-package.sh`, sourced by both
`build-windows-sdk.sh` and `build-windows-sdk-native.sh`. It holds exactly the
logic that does not care which environment produced the linked `migo.dll`:

- `.def` export-allowlist generation from `include/migo/*.h`
- staging the prefix (`include/`, `lib/`, `bin/`, `lib/cmake/migo/`)
- writing the CMake `find_package` config and version file
- writing `share/migo/windows-x86_64-manifest.json` (runtime-dependency checksums,
  artifact hashes, `known_gaps`)

This is the one non-optional refactor in this design, not an incidental cleanup.
Leaving ~150 lines of manifest-writing logic duplicated between two entry scripts
is exactly the failure class this session spent the whole day fixing —
version-contract drift, vocabulary drift, the snapshot-fingerprint surprises — a
manifest fix applied to one copy and not the other is a silent, later-discovered
defect, not a hypothetical one.

`build-windows-sdk.sh`'s externally-visible behavior (CLI, env vars, output) does
not change; this refactor only moves where its tail logic lives.

### D4 — CI toolchain setup

- `ilammy/msvc-dev-cmd@v1` loads vcvars (MSVC compiler/linker/`INCLUDE`/`LIB`) into
  the job environment once, before both the `cargo build` and `link.exe` steps —
  replacing `build-windows-sdk.sh`'s `vswhere` probing and hand-built PATH
  allowlist, neither of which is needed on a single-purpose runner with no NDK on
  `PATH` to shadow anything.
- `choco install ninja -y --no-progress`, guarded by a `command -v ninja` check
  first — a safety net, not a default assumption. `skia-safe`'s `binary-cache`
  feature downloads a prebuilt `skia-bindings` for host targets and normally
  succeeds, but it 404's when a given commit's prebuilt is unavailable (reproduced
  locally this session) and falls back to compiling Skia from source via `ninja`.
  This mirrors the precedent already in this exact workflow:
  `linux-qt-host-kit`/`host-engine-tests` install `ninja-build` for the identical
  reason. Failing 40 minutes into a build with `ninja: command not found` is worse
  than a 10-second defensive install.
- `actions/setup-python@v6` (`python-version: "3.11"`), matching every other
  `release-*` job — `build-windows-sdk-native.sh` validates the generated manifest
  with `python3 -m json.tool`.
- Everything else runs as `shell: bash` (Git Bash, bundled on `windows-latest`).
  Confirmed by inspection: `fetch-v8-archives.sh`, `fetch-windows-angle.sh`, and
  `package-sdk.sh` contain no `wslpath`/`/mnt/c`/`cmd.exe` — they are already fully
  OS-agnostic and need no changes to run natively.

### D5 — `test-windows-sdk-contract.sh`'s `run_msvc` needs environment detection

This is the one existing script that does not carry over unchanged. Its `run_msvc`
helper (used to invoke `dumpbin`/`cl` against the staged package) hardcodes the
WSL→`cmd.exe` crossing: `/mnt/c/Windows/Temp`, `wslpath -w`, batch-file dispatch.
On native Windows, after `ilammy/msvc-dev-cmd`, those tools are already directly on
`PATH` — the crossing is not merely unavailable, it's unnecessary indirection.

Fix: `run_msvc` gains environment detection —
`command -v wslpath >/dev/null 2>&1`, the same capability the function's WSL body
already depends on, so the check is tied to the exact thing being branched on
rather than an incidental environment marker — and picks one of two bodies: the
existing `cmd.exe`-dispatch path unchanged when `wslpath` is present, a direct
invocation when it is not. This changes one function, not the whole file: the rest
of the contract (`dumpbin /EXPORTS` parsing, the `.def` cross-check, the
header-compiles-standalone probe) is already environment-agnostic and stays as-is.
Both `build-windows-sdk.sh` (WSL, self-invokes this at the end) and the new CI job
(invokes it as an explicit step, see D6) depend on this fix.

This does not contradict D2's "two scripts, no branching" call. D2 is about the
top-level build *orchestration* — compiling, linking, and staging are different
enough operations in the two environments that interleaving them in one control
flow would tangle two execution models together. `run_msvc` is a single, narrow
utility ("run this one MSVC tool and capture its output") where the WSL and native
bodies are two short, self-contained implementations of the same one-line
contract — branching inside one function is the simpler shape here, the same way
`snapshot_target_triple()` branches on `(os, arch)` inside one function elsewhere
in this codebase rather than existing as N near-duplicate functions.

### D6 — Release integration: zero special-casing in `publish`

`release-windows` stages into `dist/release` via `package-sdk.sh` (same call shape
as `release-linux`) and uploads with `actions/upload-artifact@v4` as
`release-assets-windows`. `publish`'s asset-collection step
(`download-artifact` with `pattern: release-assets-*`, `merge-multiple: true`) is
already fully name-driven — confirmed by reading the current job — so the *only*
change `publish` needs is the added `needs: [..., release-windows]`. No new
special-casing, no asset-list literal to keep in sync (that pattern was already
rejected once in this file's history, per its own comments, for producing exactly
the kind of drift this whole design avoids).

Concretely, the new job's step list (mirroring `release-linux`'s shape, the
closest existing analog — single target triple, staticlib→shared-lib link,
package+manifest+contract):

```yaml
release-windows:
  needs: [quality-gate, host-engine-tests]
  runs-on: windows-latest
  timeout-minutes: 90
  steps:
    - Checkout (actions/checkout@v5, lfs: false)
    - Setup Python (actions/setup-python@v6, python-version: "3.11")
    - Setup Rust toolchain (dtolnay/rust-toolchain@1.95.0, targets: x86_64-pc-windows-msvc)
    - Setup MSVC dev environment (ilammy/msvc-dev-cmd@v1)
    - Install ninja if absent (choco install ninja -y --no-progress)
    - Fetch and verify the windows-msvc V8 archive (fetch-v8-archives.sh x86_64-pc-windows-msvc)
    - Fetch and verify the pinned ANGLE runtime (fetch-windows-angle.sh)
    - Build the Windows SDK (build-windows-sdk-native.sh)
    - Windows SDK package contract (test-windows-sdk-contract.sh --strict)
    - Package the Windows SDK for release (package-sdk.sh dist/migo-windows-x86_64 --output-dir dist/release)
    - Upload the staged Windows assets (actions/upload-artifact@v4, name: release-assets-windows)
```

`test-windows-sdk-contract.sh` needs no `MIGO_WINDOWS_PREFIX` override: it already
defaults to `dist/migo-windows-x86_64`, matching `build-windows-sdk-native.sh`'s
own default `PREFIX`.

### D7 — Retire the stale "built by hand" language

Once this ships, two comments become actively misleading and get corrected as part
of this change (touching code this design already edits, not a drive-by):
`build-windows-sdk-native.sh`'s manifest `known_gaps` entry claiming no CI job
exists, and `publish`'s job-level comment in `release.yml` that still frames
`verify-release-assets.sh` as "what a human runs by hand after uploading the
Windows and OpenHarmony packages" — stale since OHOS joined CI earlier in this
session; now doubly stale once Windows does too.

## Testing / verification plan

- Push the branch and watch the actual `release-windows` job run on
  `windows-latest` — this design lives or dies on real CI mechanics
  (`ilammy/msvc-dev-cmd` actually loading vcvars for a bash step, `ninja` actually
  installing, the fetch scripts actually working un-translated), which cannot be
  verified any other way from this machine.
- Confirm the produced `migo-0.9.2-capi-windows-x86_64.tar.gz`'s
  `package_sha256` matches what `build-windows-sdk.sh` (WSL path) produces for the
  same commit — same source, same V8 archive, same ANGLE pin, so a match proves
  the native path is not silently linking something different.
  `scripts/test-sdk-package-reproducibility-contract.sh` already exists for the
  general property; this is a specific instance of it.
- Confirm `verify-release-assets.sh <tag>` reports the Windows asset covered by
  `SHA256SUMS.txt`, not its own sidecar — the concrete, checkable definition of
  "no longer built by hand."
- Confirm the local WSL flow (`build-windows-sdk.sh`) still produces a passing
  package after the D3 refactor — same command, same output, proving the
  extraction didn't change behavior for the caller that already worked.

## Rollout

This is Phase 1 of 3. Phase 2 (Linux arm64) and Phase 3 (Windows arm64) are
separate, independently-specced projects — Phase 3 in particular needs upfront
research into GitHub-hosted Windows-arm64 runner availability before it can even
be scoped, let alone specced.
