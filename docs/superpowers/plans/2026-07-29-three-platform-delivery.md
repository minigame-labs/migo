# Three-Platform Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the approved three-platform delivery specification into a locally verified `0.10.0-rc.1` release candidate for Android, Linux, and Windows without publishing it.

**Architecture:** Correct runtime ownership and platform invariants first, then make every native input and build product hermetic, qualify the three native platforms, harden the examples and service boundaries, and finally produce performance evidence plus a deterministic release-candidate bundle. Each phase is fail-closed: a later phase cannot compensate for an unverified earlier invariant.

**Tech Stack:** Rust 2024, C/C++ ABI, JNI/Gradle, CMake/Ninja, Bash/PowerShell, GitHub Actions, V8, Skia, ANGLE, EGL, X11, Wayland, Win32, Android NDK, Syft/CycloneDX.

---

## Execution Rules

- The design authority is
  `docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md`.
- Work happens on `delivery/three-platform-rc` in the
  `.worktrees/three-platform-delivery` worktree.
- Every checkbox is backed by a test, build result, package audit, or an
  explicitly documented external qualification requirement.
- Use test-driven development for behavior changes and systematic debugging for
  every unexpected failure.
- Commit completed, verified stages locally. Do not push, tag, create a GitHub
  release, or publish an artifact.
- Missing local tools, SDKs, native target environments, physical devices, or
  signing credentials are recorded with exact installation or execution
  commands. They are not converted into passing skips.
- A platform is not deliverable until its clean native lane passes. Cross-checks
  are useful evidence but never replace Android, Linux, or Windows native
  qualification.

## Delivery Ledger

### Phase A: Correctness Foundation

- [ ] A1: Own and join every Host thread; make Engine destruction the final
  lifecycle barrier.
  Detailed plan:
  `docs/superpowers/plans/2026-07-29-a1-owned-host-lifecycle.md`.
- [ ] A2: Add immutable `PlatformIdentity` and reject incompatible reattachment
  synchronously before attachment publication.
  Detailed plan:
  `docs/superpowers/plans/2026-07-29-a2-platform-identity.md`.
- [ ] A3: Move X11 rendering to a Migo-owned connection and remove the
  undocumented `XInitThreads` precondition.
  Detailed plan:
  `docs/superpowers/plans/2026-07-29-a3-owned-x11-connection.md`.
- [x] A4: Make terminal input transitions lossless under bounded saturation and
  coalesce only replaceable motion.
  Detailed plan:
  `docs/superpowers/plans/2026-07-29-a4-lossless-input-state.md`.
- [ ] A5: Correct desktop pointer/button semantics, including Qt hover and
  pressed-button state.
- [ ] A6: Run lifecycle, reattachment, input saturation, ABI, and header
  contract suites with both product profiles.
- [ ] A7: Complete Android capability enforcement and revocation. Every one of
  the 30 protected operations and 8 cleanup operations must have an exhaustive
  runtime classification and an exact Android service-layer classification;
  revocation, deferred framework entry, and Session shutdown share one
  fail-closed ordering. Resource teardown must be exception-safe, observable,
  and retryable without reporting false success. Full/Slim merged manifests
  must satisfy the API 26/28/31 matrix in Phase B9.
- [ ] A8: Finish and independently review the retained-intrinsic host bridge,
  mounted-module URL validation, ad-event authority, late callback rejection,
  and the reliable asynchronous host-result lane. Behavioral tests must prove
  that bounded input pressure cannot drop a terminal result and that closed
  Sessions retain no Java handler or sink.
- [ ] A9: Make runtime restart a callback and resource ownership boundary.
  Execute the detailed plan in
  `docs/superpowers/plans/2026-08-02-runtime-restart-generation-boundary.md`.
- [ ] A10: Correct Canvas recovery as one transactional resource operation:
  preserve or deliberately reset the complete Canvas2D save/clip/path state,
  rematerialize pattern image resources, keep explicit main-canvas dimensions
  consistent with the native backing store, and restore GL state plus temporary
  allocations on every snapshot-blit failure path. Tests must inject each
  failure and exercise the real manager recovery path.
- [ ] A11: Complete the permission product contract. Add a public Session API
  that seeds and updates standing host decisions before content startup; source
  authorization descriptions only from validated `game.json` declarations;
  remove stale `_authSetting` reads; and prove `wx.getSetting`, `wx.authorize`,
  album write, location, user info, camera, recorder, and Bluetooth agree after
  grant, denial, revocation, restart, and Session close.
- [ ] A12: Close remaining platform identity and reporting gaps: reject zero,
  negative, non-finite, and otherwise invalid host pixel ratios; canonicalize
  Windows game identity with the same rules as the other platforms; and make a
  missing ad handler settle the content-visible request through its documented
  error path rather than leaving it pending.
- [ ] A13: Re-run the complete post-master integration audit. Resolve every
  conflict using the current public API/profile baselines, regenerate only the
  snapshots whose verified fingerprints changed, and record the exact Full and
  Slim test counts. No source-text contract may substitute for the behavioral
  lifecycle, callback-race, Canvas, or resource-teardown tests above.

Phase A exit evidence:

A4 evidence recorded 2026-07-29: the four affected Rust crates passed their
library tests and all-target checks; Android Full/Slim unit tests and lint
passed; input, surface, and C ABI static contracts passed. A1-A3 and A5-A13
remain separate exit requirements. A7-A13 were added after reviewing the
subsequent `master` commits and are mandatory release blockers, not follow-up
enhancements.

```bash
cd engine
cargo test -p migo-core --lib --locked --offline
cargo test -p migo-capi --lib --locked --offline
cargo test -p migo-platform --lib --locked --offline
cd ..
bash scripts/test-c-abi-surface-candidate.sh
bash scripts/test-surface-attachment-contract.sh
bash scripts/test-product-profiles.sh
```

### Phase B: Hermetic Cross-Platform Builds

- [ ] B1: Add one repository release-version source and verify consistent
  propagation to Cargo, Gradle, CMake, archives, manifests, and documentation.
- [ ] B2: Materialize V8/Skia/ANGLE/native inputs under immutable,
  content-addressed paths before any Cargo or native link invocation.
- [ ] B3: Remove every global `--allow-multiple-definition` relaxation and
  resolve duplicate C++ runtime symbols at component build boundaries.
- [ ] B4: Make release metadata and archives deterministic under
  `SOURCE_DATE_EPOCH`.
- [ ] B5: Repair Android PowerShell packaging and prohibit release
  `--skip-rust` paths.
- [ ] B6: Replace the Windows warm-target link flow with a clean,
  Windows-native MSVC/ANGLE build graph.
- [ ] B7: Produce deterministic Android, Linux, and Windows packages containing
  manifests, checksums, BSL 1.1 license text, notices, SBOMs, and build
  provenance.
- [ ] B8: Prove same-source rebuild equality for every shipping archive.
- [ ] B9: Parse the built Full and Slim merged manifests and verify the Android
  permission contract at API 26, 28, and 31, including each capped
  declaration's effective API boundary.

Phase B exit evidence:

`scripts/test-android-release-manifest-gate.sh` must inspect merged manifests
from the actual Full and Slim artifacts, not infer results from source-manifest
text. Its permission evaluator must make these assertions:

- API 26 Full: coarse and fine location are effective and have no
  `maxSdkVersion`; legacy `BLUETOOTH`/`BLUETOOTH_ADMIN` are effective with
  `maxSdkVersion="30"`; `WRITE_EXTERNAL_STORAGE` is effective with
  `maxSdkVersion="28"`.
- API 28 Full: coarse and fine location remain effective without a cap;
  legacy Bluetooth remains effective; `WRITE_EXTERNAL_STORAGE` is effective at
  its inclusive upper boundary of 28.
- API 31 Full: coarse and fine location remain effective without a cap;
  `BLUETOOTH_SCAN` and `BLUETOOTH_CONNECT` are effective; legacy Bluetooth and
  `WRITE_EXTERNAL_STORAGE` declarations are ineffective because their caps
  have passed, and album write relies on `MediaStore` without storage
  permission.
- API 26, 28, and 31 Slim: the merged manifest contains none of the Full-only
  dangerous capability permissions for camera, microphone, location, BLE, or
  legacy album write. Shared normal permissions are checked separately and do
  not satisfy this assertion.

```bash
bash scripts/test-android-release-manifest-gate.sh
bash scripts/test-android-sdk-contract.sh
bash scripts/test-linux-sdk-contract.sh
pwsh -File scripts/test-windows-sdk-contract.ps1
bash scripts/test-release-asset-ordering-contract.sh
```

The platform-native build commands and exact package paths are added to the
phase plan from the final build-driver interfaces introduced in B1-B7, then
recorded verbatim in the release-candidate evidence directory.

### Phase C: Native Platform Qualification

- [ ] C1: Build the Android API 26+ full and slim AARs plus C SDK from a clean
  tree for `arm64-v8a` and `x86_64`.
- [ ] C2: Run Android emulator smoke tests for both ABIs and a physical
  arm64-v8a lifecycle/input/surface stress suite.
- [ ] C3: Build the Linux glibc x86_64 shared/static SDK and player from a clean
  tree; validate X11, Wayland, Qt integration, resize, input, and teardown.
- [ ] C4: Build the Windows 10/11 x86_64 DLL/import library/static SDK and
  player natively with MSVC/ANGLE; validate Win32 resize, input, and teardown.
- [ ] C5: Compile and execute clean C/C++ consumer projects against installed
  packages on all three platforms.
- [ ] C6: Archive native logs, compiler identities, runtime versions, first-frame
  evidence, and thread-clean teardown evidence.

Phase C exit condition: every supported platform reaches first frame, handles
surface recreation/resize and input, and exits with no live Migo-owned thread.

### Phase D: Examples, Security, And Developer Experience

- [ ] D1: Align `../migo-examples` with the candidate package layouts and
  version contract without repository-relative SDK assumptions.
- [ ] D2: Correct Android touch flags, Embedded content IDs, and every sample
  lifecycle path; prove session/engine cleanup.
- [ ] D3: Correct Linux X11/Wayland/Qt and Windows Win32 sample input and
  lifecycle behavior.
- [ ] D4: Replace resolver sidecar self-assertion with a configured trust root,
  canonical signed metadata, expiry checking, and fail-closed verification.
- [ ] D5: Bind the authentication relay to loopback by default; add explicit
  origin allowlisting, per-session credentials, bounded payloads, timeouts, and
  redacted logs.
- [ ] D6: Remove shell-command construction from the Windows runner and pass
  structured arguments directly to child processes.
- [ ] D7: Add Linux, Windows, and Android example CI lanes with no success-masking
  operators.
- [ ] D8: Verify a clean examples checkout resolves and consumes only candidate
  SDK artifacts and reaches first frame on every supported host.

Phase D exit condition: all examples are secure reference integrations, not
merely demos that happen to start in a developer checkout.

### Phase E: Performance And Release Candidate

- [ ] E1: Repair product-profile tests so exact filters execute one or more
  intended tests in both full and slim configurations.
- [ ] E2: Make device/performance collection build, install, launch, sample,
  validate required fields, and fail closed.
- [ ] E3: Run the specified representative workloads on a physical Android
  device against Migo and the WebView baseline; record FPS/frame-time, startup,
  memory, CPU, thermal, energy, and artifact-size evidence.
- [ ] E4: Create native Linux and Windows smoke/performance evidence sufficient
  to detect platform-specific regressions.
- [ ] E5: Rewrite root, platform, build, contributor, security, compatibility,
  changelog, and examples documentation for the source-available product and
  verified delivery matrix.
- [ ] E6: Assemble the complete `0.10.0-rc.1` artifact matrix locally.
- [ ] E7: Verify checksums, signatures/provenance inputs, SBOMs, licenses,
  notices, exports, ABI floors, archive reproducibility, install consumers, and
  native smoke evidence from the assembled directory.
- [ ] E8: Run an independent final code/document/package review and close every
  release-blocking finding.
- [ ] E9: Commit the verified release-candidate state locally without pushing,
  tagging, or publishing.

Phase E exit condition: every statement in specification Section 14 has direct
evidence, and the release workflow would be able to publish only these verified
bytes if publication were later authorized.

## Plan Authoring Sequence

Before editing a phase, create and commit its detailed test-driven plan under
`docs/superpowers/plans/`. A detailed plan names exact files, tests, commands,
expected failures, implementation boundaries, and commit points. Split a phase
when independent invariants need separate rollback or review boundaries; never
combine fixes solely to reduce commit count.

## Final Verification Record

The final candidate stores non-shipping verification evidence under
`build/release/0.10.0-rc.1/evidence/` while the durable source-controlled
release procedure and expected schemas live under `docs/` and `contracts/`.
Generated binaries and device logs remain uncommitted unless an existing
repository contract explicitly versions the artifact.
