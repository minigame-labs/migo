> Part of the [Four-Platform Delivery Ledger](../2026-08-03-four-platform-delivery.md).

## Phase 2 — Native Platform Qualification

- [ ] 2.1 Build Android Full and Slim AARs plus the C SDK from a clean tree for
  `arm64-v8a` and `x86_64`.

  **Partial, and only the Full debug half — 2026-08-10.** `bash scripts/build-aar.sh debug
  arm64-v8a x86_64` produces `migo-full-debug.aar` (197,932,115 bytes) with both ABIs, the
  embedded artifact manifests verified at package time and a release attestation emitted.
  Not done here: **Slim**, **release**, the **C SDK**, and **from a clean tree** — this ran
  against a warm workspace. It was built to unblock 2.2, so it is evidence for that item's
  rendering row and not for this one.

  **A packaging rule this exposed, worth stating because it makes an obvious shortcut
  unsafe.** Building a single ABI incrementally does not work once another ABI's libraries
  are staged: `engine/jniLibs/full/` accumulates per-ABI directories, Gradle packages every
  one it finds, and the artifact manifest indexes only what the current invocation built.
  The gate then refuses the package by name —

  ```
  AAR manifest verification: AAR has unindexed Migo JNI entries:
    ['jni/arm64-v8a/libc++_shared.so', 'jni/arm64-v8a/libmigo.so']
  ```

  — and **produces no AAR at all**, which is the right outcome: an archive carrying a
  native library its own manifest does not describe is precisely what the manifest exists
  to prevent, and the failure is louder than a package that quietly ships one. The
  consequence for this item is that a shipping build must name every ABI it intends to
  publish in one invocation; `build-aar.sh debug x86_64` after an `arm64-v8a` build is not
  a smaller version of the same thing, it is a build that fails.
- [ ] 2.2 Run Android emulator smoke tests for both ABIs and the physical
  `arm64-v8a` lifecycle, input, and surface stress suite.

  **Rendering conformance passes on the physical device at this HEAD — 2026-08-10.**
  Mate30 Pro (TAS-AN00, `arm64-v8a`, SDK 31), running an AAR built from this tree
  (`migo-full-debug-arm64-v8a.aar`, 98,136,703 bytes, embedded artifact manifests
  verified at package time):

  ```
  android: 33 assertions, 33 passed, 0 known-failing, 0 new, 0 fixed-but-listed
  ```

  Every assertion carries its measured pixels rather than a self-report — e.g.
  `canvas2d/fill-rect-covers-exactly … at (20,20) want [255,0,0,255] +/-2, got
  [255,0,0,255]` — and `expectations/android.txt` is empty of IDs, so this is a clean
  sheet and not a suite grading itself against a list of tolerated failures. The suite's
  own two rules make the number trustworthy: **a run producing no `DONE` line is a
  failure, not a pass**, and **an ID listed as known-failing that passes is also a
  failure**, so neither an unloaded bundle nor a stale exemption can be read as green.

  It matters that this ran on an AAR built *after* PR #35: the bench shell was carrying a
  1 August build, which predates the runtime generation fence, so the previous numbers
  said nothing about the current engine.

  **Both ABIs pass, against one package — 2026-08-10.** The suite was then run on the
  `x86_64` emulator (AVD `migo-api26`, API 26, KVM-accelerated, `swiftshader_indirect`)
  and re-run on the device, both against the *same* dual-ABI
  `migo-full-debug.aar` (197,932,115 bytes), so the two rows are comparable rather than
  two builds each tested once:

  ```
  emulator x86_64: 33 / 33   new_failures 0
  device   arm64  : 33 / 33   new_failures 0
  same assertion set: True (33 ids)
  ```

  The assertion sets are compared, not just the totals — two runs of 33 could otherwise be
  33 *different* things. And the software rasteriser matched **exactly**, not merely within
  tolerance (`reftest-rect-equals-path max channel delta 0 <= 1`), so the Canvas2D results
  do not depend on Mali-specific behaviour.

  **The suite cannot hold both results at once, which had to be worked around by hand.**
  Both runners write `dist/results-android.json` and stamp `platform: "android"` regardless
  of whether the target is an emulator or a phone, so the second run silently overwrote the
  first. The per-assertion pixel values above survive only because each file was copied
  aside immediately after its run. That is a real gap in the suite for a two-ABI claim —
  the numbers here are trustworthy because of a manual step, which is not a property a
  gate should depend on.

  **This is one row of 2.2 and the item stays open.** What it covers is *rendering
  conformance* — canvas2d basics, canvas2d readback, surface geometry — on one ABI. What
  2.2 asks for and this does not touch:

  * **lifecycle, input, and surface stress.** There is no harness for these in any
    repository — conformance drives rendering only, and 2.2's other three words need one
    written. Attach/detach across restart, backgrounding, surface recreation, multi-touch
    saturation and teardown are all unmeasured here. They also cannot be a self-contained
    bundle the way the rendering tests are: the stimulus has to come from the host (adb
    `keyevent`, `force-stop`, rotation), so the shape is external driver + engine
    self-reported event sequence + expected-sequence comparison, reusing the existing
    `[conformance]`/`DONE` verdict channel rather than inventing a second one.

  Calling 33/33 "the device suite passes" would be exactly the overclaim this ledger keeps
  catching: it is the rendering suite, and it passes.
- [ ] 2.3 Build the Linux shared and static SDK and player from a clean tree;
  validate X11, Wayland, Qt, resize, input, and teardown.
- [ ] 2.4 Build the Windows DLL, import library, and static SDK natively with
  MSVC and ANGLE; validate Win32 resize, DPI change, input, and teardown.

  **The build half is done — 2026-08-10.** `bash scripts/build-windows-sdk.sh` runs
  end to end on a workstation carrying VS Build Tools (MSVC 14.44.35207, Windows SDK
  10.0.22621 and 10.0.26100, LLVM at the default path). `migo-capi` compiles for
  `x86_64-pc-windows-msvc` in 30.47s release, and the manual link produces
  `migo.dll` (26,422,272 bytes) plus the import library `migo.lib`, staged into
  `dist/migo-windows-x86_64` with headers, the CMake package, and the ANGLE and V8
  runtime DLLs beside it. `scripts/test-windows-sdk-contract.sh` then passes all six
  checks with **0 skipped**:

  ```
  PASS  package carries the DLL, import library, headers and CMake package
  PASS  ANGLE and V8 runtime DLLs ship alongside migo.dll
  PASS  export surface is exactly the declared migo_* set (24 entries)
  PASS  migo.dll loads and reports it can attach a Win32 HWND
  PASS  staged headers compile standalone under MSVC C11 (/W4 /WX)
  PASS  import library references migo.dll
  ```

  `0 skipped` is load-bearing here: this contract is the kind that can pass by
  finding nothing, and it states the count so an empty run and a clean run are
  distinguishable.

  **This is not most of 2.4, and the item stays open.** Three of its four clauses are
  untouched:

  * **the static SDK is not built.** `migo_capi.lib` exists only as the staticlib
    input to the DLL link; nothing packages or verifies a static consumer. The
    arrangement is also not free to change — `windows-v8.lock.json` records that V8
    must be absorbed through its *import* library, because linking the static archive
    puts V8's bundled libc++ into the same link as Skia's MSVC STL and the two define
    `std::terminate` incompatibly. A static SDK therefore needs that collision
    answered, not just a second `cargo build`.
  * **resize, DPI change, input and teardown are not validated.** Nothing has been
    driven through a live `HWND`. "`migo.dll` loads and reports it can attach a Win32
    HWND" is a capability report, not a frame, and it must not be read as one.
  * **it was not built from a clean tree**, which this item explicitly asks for. The
    ordering defect that made a cold target *impossible* is fixed in 1.7 — the search path
    is now derived after the build, and a missing one fails by name instead of as a bare
    `LNK1181` — but the fix was verified on a warm target plus an injected failure, not by
    an actual empty-`CARGO_TARGET_DIR` run, which means rebuilding Skia. So "the mechanism
    no longer prevents it" is proven and "it builds from a clean tree" is still
    NOT PROVEN. Those are different sentences and this item needs the second one.
  * NuGet packaging remains, which the script says itself.

  Neither independent review has run: no codex on this workstation, waived by the
  operator rather than satisfied.
- [ ] 2.5 Qualify HarmonyOS on the `x86_64` emulator: attach, content-ready,
  first frame, resize, background and foreground, surface recreation,
  multi-touch, audio playback, detach, shutdown.
- [!] 2.6 Qualify HarmonyOS `arm64-v8a` on a physical device for the same
  behaviour set. **Blocked on hardware.** No HarmonyOS NEXT device is available.
  Until one is, `arm64-v8a` may be built, symbol-audited, and emulator-verified,
  but must not be announced delivery-ready, and its performance row stays empty
  rather than being filled from emulator data.
- [ ] 2.7 Compile and execute clean C and C++ consumer projects against the
  installed packages on all four platforms.
- [ ] 2.8 Archive native logs, compiler identities, runtime versions,
  first-frame evidence, and thread-clean teardown evidence.

## Phase 3 — Integration Contract

- [ ] 3.1 Define the canonical host callback set at the C ABI. Add one versioned
  registration structure and entry point per product-level capability — message,
  auth, permission, ad, log, subpackage — each carrying `struct_size`,
  `abi_version`, its function pointers, and its `user_data`. Do not disturb the
  existing `MigoHostCallbacks` layout or its static assertions. Permission and
  ad are handler-plus-sink so a host may resolve them asynchronously.
- [ ] 3.2 Specify and test the unregistered path for every product-level
  capability: a content request with no registered handler settles through its
  documented error path, never hangs, never returns false success, never stays
  pending.
- [ ] 3.3 Add the header-only C++ RAII desktop wrapper with no GUI-toolkit
  dependency, covering session construction, surface attach and rebuild, input
  forwarding, the canonical callback set, and deterministic teardown honouring
  the documented release barriers.
- [ ] 3.4 Make the Qt 6 host kit cross-platform: Linux X11, Linux Wayland, and
  Windows, moved out of the Linux-only location.
- [ ] 3.5 Build the HarmonyOS HAR with its ArkTS facade and carry the canonical
  callback set across the NAPI bridge in both directions. HarmonyOS has no
  inbound callback path today, so every callback is new work.
- [ ] 3.6 Confirm the Android facade exposes the canonical set and adds no
  capability the C ABI lacks.
- [ ] 3.7 Add the integration parity gate: per platform and tier, measure
  integration-region line count and distinct Migo API calls, commit the
  baselines, and fail on regression, on a facade exceeding one quarter of the
  Tier 0 line count, on a missing canonical callback, on a registered capability
  with no reachability test, or on an unregistered capability with no settlement
  test.

## Phase 4 — Consumers And Helper Repositories

- [ ] 4.1 `migo-examples`: collapse the three per-platform pins to the unified
  version, add the HarmonyOS example covering both the HAR consumer and the C
  ABI consumer, add a Tier 0 example on every platform to anchor the parity
  ratio, and add the integration-region markers.
- [ ] 4.2 `migo-examples`: correct Android touch flags and embedded content IDs,
  Linux X11/Wayland/Qt and Windows Win32 input and lifecycle behaviour, and
  prove session and engine cleanup.
- [ ] 4.3 Replace resolver sidecar self-assertion with a configured trust root,
  canonical signed metadata, expiry checking, and fail-closed verification.
- [ ] 4.4 Bind the authentication relay to loopback by default with explicit
  origin allowlisting, per-session credentials, bounded payloads, timeouts, and
  redacted logs.
- [ ] 4.5 Remove shell-command construction from the Windows runner.
- [ ] 4.6 `migo-conformance`: add Windows and HarmonyOS runners, add
  rasterisation golden references with a declared tolerance, and make
  expectation files present and attributable.
- [ ] 4.7 `migo-bench`: extend to four platforms with the System WebView,
  ArkWeb, Chromium, and WebView2 baselines; key every baseline to the unified
  product version rather than a git SHA; cover high, mid, and low mobile device
  tiers.
- [!] 4.8 `migo-test-suite`: commit the six untracked render specifications and
  record baselines carrying date, device, platform, and Migo revision. **Blocked
  on a decision:** the render specifications are untracked local work and the
  baseline directories are empty scaffolding, so the suite cannot gate anything
  until its author commits them.
- [ ] 4.9 `migo-android-demo`: replace the hard-coded
  `../../migo/platforms/android/dist/migo-debug.aar` path with resolution of a
  released artifact at the unified version.
- [ ] 4.10 Add Android, Linux, Windows, and HarmonyOS example CI lanes with no
  success-masking operators.

## Phase 5 — Performance And Release Candidate

- [ ] 5.1 Implement the six structural performance requirements from
  specification Section 7.3 with a regression test each: bounded hot paths, zero
  steady-state allocation, no cross-session lock on a per-event path, idle
  quiescence with a per-platform wakeup ceiling, no redundant presentation copy,
  and no steady-state growth.
  **Idle quiescence's behavioural half closed under task 0.50** — the engine-paced
  clock is demand-driven and measured at 0 wakeups per second at idle against 59
  before. What stays here is the *ceiling*: a per-platform value in the versioned
  threshold file, and the same measurement run on Windows and HarmonyOS hosts
  rather than the Linux host alone.
  **No steady-state growth got its mechanism under task 0.51** — a net-live-bytes
  cycle gate plus a resident-memory measurement over a long workload, both
  two-sided. ~~What stays here is likewise the threshold, and gates for the cycles
  that measurement cannot reach: session create/destroy, the V8 heap across a soft
  restart, and GPU-side growth.~~ **Session create/destroy now has its gate**, in
  `capi/src/concurrent_sessions.rs`: the C API creates and destroys a real Session 64
  measured times against a counting allocator, and the cycle nets non-positive. The
  process measurement cannot reach it because that workload renders and never creates a
  Session.

  Getting it attributed took the rule about redundancy. The obvious mutant —
  `mem::forget` of the exported `Arc` — kills the gate **and** two pre-existing tests
  that watch the handle's own strong count, so it is the same claim at two levels. The
  case only the gate can see is a leak that count is blind to: a Session owns a heap
  block that is not the handle's own, `pending_surface_releases`, and an extra clone of
  that inner `Arc` escaping the Session fails the gate and nothing else in the crate's
  143 tests. That is also the realistic shape for this field, since the asynchronous
  surface-release path is exactly what wants a handle outliving a call.

  The result is a finding rather than a fix: the cycle does not grow. What stays open on
  this bullet is the threshold, the V8 heap across a soft restart, GPU-side growth, and
  anything a Session registers process-wide once it has a surface — a surfaceless
  Session has no Host and so no isolate, no text cache entry and no stats registration,
  which is why the gate's reach stops where it does.
  **The cross-session lock requirement is now gated on both sides, and the design
  recorded here was not the one used.** It is not satisfied by the declaration guard
  added under task 0.1: that guard reflects on the session map's declared type, and an
  independent review constructed a counterexample it cannot detect — taking the open
  guard inside the lookup leaves the declared type unchanged and releases the lock
  before any observable barrier, so both delivered fixtures still pass. ~~The intended
  behavioural design is to enable JVM thread contention monitoring, drive concurrent
  admissions across several sessions, and assert that the blocked count attributable to
  admission is zero.~~ **That was rejected on this project's own rule about absence
  metrics**: a run in which nothing was admitted also blocks for zero milliseconds, and
  making the number the pass condition is the shape of gate this plan keeps catching.

  What landed instead is the Rust probe's shape, in Java: hold `openGuard` and require
  `runIfGranted` — the admission a BLE characteristic notification takes — to complete
  on another thread anyway. Contention is manufactured rather than waited for, so an
  *uncontended* acquisition fails it too, which is what a load test cannot see. The
  guard is package-private rather than reached by reflection, because reflection would
  go on compiling after a rename. Three details are load-bearing: the admission runs on
  another thread, since Java monitors are reentrant and the holder's own thread would
  pass either way; saturation is asserted before the admission starts; and the
  callback's return value is asserted, because a refused admission returns instantly and
  would satisfy the timing assertion without reaching the lookup.

  **The instrument has its own control, and it is the reason the pair is a gate.**
  `perEventAdmissionDoesNotWaitForTheAdmissionGuard` asserts an *absence*, which is
  satisfied by a guard nobody held and by a monitor the test failed to acquire. So
  `openingASessionDoesWaitForTheAdmissionGuard` requires the operation that genuinely
  takes the guard to stay blocked for the same held guard. Its bound is the short one,
  because a correct `open` can never complete while the guard is held and so cannot
  flake.

  | Mutant | Kills |
  | --- | --- |
  | `runIfGranted`'s lookup takes `openGuard` — the review's own counterexample | `perEventAdmissionDoesNotWaitForTheAdmissionGuard` |
  | `open` synchronizes on a fresh monitor instead of the shared one | `openingASessionDoesWaitForTheAdmissionGuard` |

  Neither kills the other's test, so the property and the instrument are separately
  pinned. 106 tests per flavour, Full and Slim, from a 104 baseline, no failures,
  errors or skips; `scripts/test-permission-coverage-contract.sh` still 30 gated, 8
  cleanup, 38 sensitive.

  **Still open on this bullet:** removing per-event allocation from the BLE callback
  path via a counted attempt admission with an interned connection wrapper. Its Rust
  half is measured at five allocations per notification (task 0.26) and is
  `cfg(target_os = "android")`; its Java half needs a JVM allocation mechanism, which
  `platforms/android` still has none of.
- [ ] 5.2 Make device and machine performance collection build, install, launch,
  sample, validate required fields, and fail closed on every platform.
- [ ] 5.3 Run the representative workloads against each platform baseline and
  record startup, frame time, memory, CPU, thermal, energy, and artifact size.
  HarmonyOS device rows depend on item 2.6.
- [ ] 5.4 Rewrite root, platform, build, contributor, security, compatibility,
  changelog, and examples documentation, including the two-path HarmonyOS
  coverage statement and the platform-neutral canonical callback documentation.
- [ ] 5.5 Assemble the complete `0.10.0-rc.1` artifact matrix locally.
- [ ] 5.6 Verify checksums, provenance inputs, SBOMs, licences, notices,
  exports, ABI and API floors, archive reproducibility, installed consumers,
  conformance results, and native smoke evidence from the assembled directory.
- [ ] 5.7 Run the final independent audit, close every finding, and commit the
  verified candidate locally without pushing, tagging, or publishing.

## Blocked Item Details

### 1.1 Android `aarch64` V8 archive

Resolved. Retained here only as the reproduction recipe, since the environment
work is not committed to this repository:

```bash
# gn >= 2315 is required; chrome-infra download is blocked, so build it.
# A shallow clone breaks gen.py, which derives the version from
# `git describe --match initial-commit`, so clone with full history.
cd /data/work/opensource && git clone https://gn.googlesource.com/gn
cd gn && CC=/usr/bin/gcc CXX=/usr/bin/g++ python3 build/gen.py && ninja -C out gn
mkdir -p /data/work/opensource/rusty_v8_src/third_party/v8_correct_gn
cp out/gn /data/work/opensource/rusty_v8_src/third_party/v8_correct_gn/gn

# Then, from the repository root:
RUSTY_V8_SRC=/data/work/opensource/rusty_v8_src \
  ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/23.2.8568313 \
  ./scripts/build-v8-android.sh aarch64
```

Building gn with the `clang++` first on `PATH` does not work here, because that
is the Android NDK's clang 12 rather than a host compiler and gn now requires
C++23. Host gcc must be selected explicitly.

### 2.6 HarmonyOS `arm64-v8a` device

Requires purchasing a HarmonyOS NEXT device. Emulator evidence never satisfies
this gate.

### 4.8 `migo-test-suite` assets

Requires the suite's author to commit the six untracked render specifications and
produce attributable baselines. Untracked local work cannot satisfy a delivery
gate.

### Linux-host portability of the build scripts

Target is Linux only, so GNU patch, GNU coreutils and bash 5 are the baseline and
are deliberately not probed — a check whose failure case cannot arise on a
supported host is noise. What does vary between Linux machines is which tools are
installed and where things live, and two real defects of that kind existed:

- `build-v8-android.sh` defaulted `RUSTY_V8_SRC` to `/home/wkspace/rusty_v8_src`,
  a path present on no current machine, while `build-v8-linux.sh` and
  `build-v8-ohos.sh` already derived theirs from the repository location. Fixed in
  task 1.1c, and `test-v8-patch-application-contract.sh` now fails on any
  `:-/data/`, `:-/home/<user>` or `:-/mnt/<drive>/` default in the V8 and gn
  scripts. Verified load-bearing: the check fires on line 37 of that file at
  commit `8a15ae6`.
- Seven scripts defaulted `ANDROID_NDK_HOME` to `$HOME/Android/Ndk`, which exists
  on none of them. Fixed for the V8 build in task 1.1a; the remaining six are
  task 1.1k.

`scripts/lib/host-requirements.sh` names the tools the V8 patch stage needs
(`patch`, `git`, `python3`) so a lean CI image or container fails by name instead
of far from the cause. `mktemp --suffix` — GNU-only, and unnecessary — was
replaced with a temporary directory.

Deliberately left alone: `build-ohos-host.sh` and `run-ohos-host.sh` default
`DEVECO_HOME` to a `/mnt/c` path. That is a documented deliberate choice, since
the emulator and hvigor live on the Windows side while the engine builds in WSL,
and both scripts fail immediately naming `DEVECO_HOME` when it is absent. A
default that degrades into an actionable error is a different thing from one that
silently resolves somewhere wrong.

Both contract tests pass from an unrelated working directory (`cd / && bash
/abs/path/test-*.sh`), so neither depends on being invoked from the repository
root.
