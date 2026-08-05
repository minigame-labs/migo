# Migo Three-Platform Delivery Design

**Status:** Approved for implementation

**Date:** 2026-07-29

**Repositories:**

- `migo`: runtime, platform integrations, packaging, release automation, and
  product documentation.
- `migo-examples`: supported integration examples and release-asset consumer
  validation.

## 1. Objective

Make Migo ready for an external release on Android, Linux, and Windows. A
release-ready revision must be correct under normal lifecycle and failure
conditions, reproducible from clean builders, performance-measured rather than
performance-claimed, consumable through tested examples, and documented as the
product that is actually shipped.

The work ends when a release candidate can be built and verified locally and by
the repository workflows. This effort does not push commits, create a tag, or
publish a GitHub Release. Commits are allowed and are expected after each
independently verified change.

## 2. Product And License Position

Migo remains licensed under Business Source License 1.1 for this delivery.
Public documentation must call the project **source-available**. It must not
call the current version OSI open source.

The product position is:

> A native game runtime for existing HTML5, Canvas, WebGL, and mini-game
> content, designed as a controllable alternative to System WebView.

Migo is a WebView alternative, not a complete WebView implementation. The
runtime core intentionally does not implement browser layout, CSS, or a full
browser DOM. The adapter exposes the tested game-facing BOM and DOM subset.
Documentation must distinguish these two facts.

## 3. Required Delivery Matrix

### 3.1 Android

- Minimum OS: Android 8.0 / API 26.
- Architectures: `arm64-v8a` and `x86_64`.
- Products:
  - Full Java/Kotlin AAR containing both ABIs.
  - Slim Java/Kotlin AAR containing both ABIs.
  - C ABI SDK for `arm64-v8a`.
  - C ABI SDK for `x86_64`.
- Native runtime: V8, Skia, Android Surface, audio, input, networking, storage,
  and the profile-specific adapter surface.
- Permission contract:
  - The Full AAR manifest declares the dangerous permissions required by its
    camera, microphone, location, BLE connection, BLE scan, and album-write
    capabilities. Because the shipping Full profile exposes `wx` location on
    API 26 and later, its `ACCESS_COARSE_LOCATION` and `ACCESS_FINE_LOCATION`
    declarations have no `maxSdkVersion`; they remain effective on API 31 and
    later. Only a declaration or product variant whose location permission is
    used solely for legacy BLE scan may cap that declaration at API 30.
  - Legacy `BLUETOOTH` and `BLUETOOTH_ADMIN` declarations are capped at API 30;
    `BLUETOOTH_SCAN` and `BLUETOOTH_CONNECT` serve API 31 and later. Album write
    on API 26 through 28 declares `WRITE_EXTERNAL_STORAGE` with
    `maxSdkVersion="28"`. API 29 and later use `MediaStore` and do not require
    or request storage permission for that path.
  - The Slim AAR does not merge those Full-only dangerous permissions into a
    consuming application. Common normal permissions remain in the shared
    manifest only when the common Java facade can exercise them.
  - Migo never requests dangerous permissions implicitly. The host owns the
    permission prompt, and every protected operation checks the current grant
    before entering the framework because grants may be revoked at any time.
  - Permission denial or revocation fails through the documented API error
    path without crashing, leaking an active scan/GATT resource, or publishing
    a false success state.
  - Basic cellular connectivity remains usable without `READ_PHONE_STATE` or
    `READ_BASIC_PHONE_STATE`; detailed radio generation degrades to `unknown`
    when the host has not granted either permission.
- Required validation:
  - Clean NDK build.
  - AAR consumer build.
  - Full and Slim merged-manifest permission assertions at API 26, 28, and 31,
    including `maxSdkVersion` boundary behavior.
  - Native CMake consumer build for each ABI.
  - API 26 symbol-floor audit.
  - Emulator startup, content-ready, first-frame, resize, background/foreground,
    Surface recreation, input, detach, and shutdown.
  - Physical-device performance and power measurements for the release gate.

### 3.2 Linux

- ABI: glibc x86_64 at the repository's declared glibc floor.
- Window systems:
  - X11 is required.
  - Wayland is required and must have the same lifecycle guarantees as X11.
- Products:
  - Versioned shared library.
  - Static library.
  - C headers.
  - CMake package.
  - pkg-config package with independently verified shared and static consumers.
  - Qt 6 host kit.
- Required validation:
  - Build against the pinned sysroot from a clean target directory.
  - Dynamic and genuinely static external consumers.
  - X11 and Wayland attach, content-ready, first-frame, resize, input, detach,
    and shutdown.
  - Loader-floor and exported-symbol audits.

### 3.3 Windows

- OS: supported Windows 10 and Windows 11 x86_64 releases.
- Toolchain: MSVC x64 with the repository-pinned Rust and LLVM toolchains.
- Window system: Win32 `HWND`.
- Graphics: ANGLE using the packaged runtime DLLs.
- Products:
  - `migo.dll`.
  - `migo.lib`.
  - C headers.
  - CMake package.
  - Required V8 and ANGLE runtime DLLs.
- Required validation:
  - Clean `windows-latest` or equivalent clean MSVC builder.
  - External CMake consumer.
  - DLL load and exact export-set audit.
  - Real HWND attach, content-ready, first-frame, resize, DPI change, input,
    detach, shutdown, and process exit with no live Migo threads.

## 4. Engineering Principles

The implementation must follow these rules:

1. Correctness precedes optimization.
2. Performance claims require repeatable measurements.
3. Required gates fail closed. Missing input, zero samples, skipped execution,
   and absent reports are failures.
4. Release builds do not consume mutable or unidentified native artifacts.
5. No release path depends on a warm Cargo, Gradle, CMake, npm, or compiler
   cache.
6. No global linker relaxation hides duplicate or unresolved symbols.
7. Public lifecycle APIs state exactly when caller-owned resources may be
   released.
8. Bounded hot paths remain bounded. Correct terminal state must not be traded
   for throughput.
9. Platform-specific behavior is isolated behind one common contract and is
   validated by a platform-native consumer.
10. Documentation, examples, manifests, and shipped bytes share the same
    version and capability sources.

## 5. Release Identity And Artifact Contract

### 5.1 Single Product Version

Add one machine-readable product version source under `release/`. The initial
release-candidate value is `0.10.0-rc.1`. Gradle metadata, CMake package
versions, package filenames, example pins, generated version JSON, and release
workflow assertions must derive from that source.

`MIGO_ABI_VERSION_CURRENT` remains the numeric C ABI negotiation version. It is
not a package version and must not be used as one.

Every release workflow must assert that the release-candidate tag is exactly
`v0.10.0-rc.1`, derived from the product-version source rather than duplicated
in workflow logic. The current work does not create that tag.

### 5.2 Artifact Matrix

One release candidate contains all of these required files:

- `migo-0.10.0-rc.1-android-full.aar`
- `migo-0.10.0-rc.1-android-slim.aar`
- `migo-0.10.0-rc.1-android-c-arm64-v8a.tar.gz`
- `migo-0.10.0-rc.1-android-c-x86_64.tar.gz`
- `migo-0.10.0-rc.1-linux-x86_64.tar.gz`
- `migo-0.10.0-rc.1-windows-x86_64.zip`
- `SHA256SUMS.txt`
- `release-manifest.json`
- `sbom.spdx.json`
- platform-specific test summaries
- platform-specific benchmark summaries where required

Each platform archive contains:

- Public headers and libraries.
- Package-manager metadata.
- `LICENSE`.
- `LEGAL.md`.
- `NOTICE`.
- Complete applicable third-party license notices.
- An embedded package manifest covering every regular file.
- Build metadata containing the exact source revision and toolchain identities.

### 5.3 Local Integrity And Published Provenance

Local packaging uses the existing artifact-manifest tool, extended to all three
platforms. The tool verifies the package tree before archive creation and the
final archive bytes after creation.

The local release candidate contains SHA-256 checksums and deterministic package
manifests. A future GitHub tag workflow additionally uses GitHub artifact
attestations or Sigstore with GitHub OIDC. An unsigned JSON file stored beside a
mutable asset is integrity metadata, not publisher authentication, and must not
be described as an attestation of identity.

Consumers use a trusted digest or a verifiable signed attestation. The examples
resolver must reject an asset that has only a same-origin unsigned sidecar when
operating in release mode.

### 5.4 Reproducibility

The release driver derives `SOURCE_DATE_EPOCH` from the source commit and passes
it to every build and packaging process. Release paths must reject an absent or
invalid value.

Archives use deterministic path ordering, ownership, permissions, timestamps,
and compression metadata. Two builds of the same source and verified component
inputs on equivalent clean builders must produce identical release archives.

Wall-clock build time may be written to a non-shipping CI log. It must not enter
classes, libraries, archives, package manifests, or release metadata.

## 6. Runtime Correctness Design

### 6.1 Thread And Destruction Ownership

The Engine and Session lifecycle must own every thread that can execute Migo
code. Spawning and forgetting a Host `JoinHandle` is not allowed.

The following guarantees are required:

- A Surface release reaching `MIGO_SURFACE_RELEASE_RELEASED` means no Migo
  thread will make a current or future access to the released native Surface or
  its native display/window resources.
- `migo_session_destroy` closes the public Session, rejects new work, drains
  callbacks according to the documented reentrancy rules, and transfers all
  remaining owned threads into an Engine-owned join set.
- `migo_engine_destroy` does not return until every Host, Render, Audio, join,
  and lifecycle worker owned by that Engine has exited.
- After `migo_engine_destroy` returns, the host may close X11/Wayland
  connections, destroy HWND resources, and unload the Migo shared library.
- A callback may request Session destruction without causing a self-join. The
  callback returns through the existing reentrant path; the Engine-owned join
  set completes the join from a different thread before Engine destruction can
  return.

Tests must use named-thread observation and host-owned sentinel resources to
prove these guarantees. Source-text tests are not sufficient.

### 6.2 Surface Reattachment

Each Graphics platform object has an immutable `PlatformIdentity`:

- Android: backend and JVM/process identity.
- X11: the Migo-owned X11 render connection identity.
- Wayland: `wl_display` identity and backend.
- Windows: ANGLE backend/device identity.

Reattaching a Surface is allowed only when the existing Host can present it with
the same `PlatformIdentity`. The compatibility check runs before attachment
state is published or a command is enqueued.

For the first delivery:

- Android Surface recreation on the same process is supported.
- X11 window replacement on the same X server is supported.
- Wayland surface replacement on the same `wl_display` is supported.
- HWND replacement under the same ANGLE platform is supported.
- X11 server replacement, Wayland display replacement, and switching between
  X11 and Wayland require a new Session and fail synchronously with the
  documented state error.

An API must never return success for a Surface that the Render thread will later
reject because its Graphics platform is incompatible.

### 6.3 X11 Threading

Migo must not depend on an undocumented call to `XInitThreads`.

At attach time, the X11 adapter resolves the server identity on the caller
thread and opens a Migo-owned X11 connection for EGL/render work. The host
continues to own and use its event-loop connection. Migo closes only the
connection it opened and does so before the relevant release guarantee is
published.

If a Migo-owned connection cannot be created, attach fails synchronously with a
specific diagnostic. No unsafe `Send` or `Sync` implementation may cite a
precondition absent from the public ABI.

### 6.4 Lossless Input State

Input transport separates coalescible state from non-coalescible transitions:

- Pointer and touch move events may be coalesced to the latest position while
  preserving stream order relative to transitions.
- DOWN/START, UP/END, CANCEL, focus-loss retractions, key-up, composition-end,
  and gamepad disconnect transitions must not be silently dropped.
- The transport remains bounded and does not allocate per event on the steady
  input path.
- Saturation is observable through metrics and an error callback, but a terminal
  transition has reserved state and supersedes older coalescible work rather
  than being discarded.
- Android Java returns `handled` only when the native layer has accepted or
  safely coalesced the event.

Tests fill every queue and mailbox to capacity, then prove that final touch,
pointer, key, composition, and gamepad state converges correctly.

### 6.5 Desktop Pointer Semantics

Qt and the adapter must preserve W3C mouse semantics:

- Hover move: `button == -1`, `buttons == 0`.
- Primary-button drag: `button == 0`, `buttons == 1`.
- Mouse up: `button == 0`, `buttons == 0`.
- Additional buttons use their standard bit masks.

The adapter derives `buttons` from the native event instead of assigning `1` to
every non-up event.

### 6.6 Runtime Restart Callback And Resource Boundary

`HostCommand::Restart` creates a new JavaScript ownership generation inside the
same native Session. A callback, result, event stream, or native resource
created for the retired generation must never resolve, mutate, or dispatch into
the replacement isolate, even when native work completes after the old isolate
has been destroyed.

The Host owns one callback-ID allocator for its entire lifetime. Every
host-bound asynchronous result and native resource whose callback is naturally
request- or handle-shaped uses an ID from this allocator. IDs are positive
`i32` values in the inclusive range `1..=i32::MAX`; they are never reset on
restart, recycled after settlement or destruction, or allowed to wrap.
Allocation is fail-closed: after `i32::MAX` is issued once, all subsequent
allocations fail before a JavaScript registry entry is created or a platform
operation is started. Main and Worker runtimes belonging to the Host share the
same allocator.

The current runtime generation is a positive signed 64-bit value owned by the
Host and mirrored to the platform. It starts at `1`, advances by exactly one at
each committed restart, never resets or wraps, and fails restart preflight at
`i64::MAX` before any revocation occurs. Rust `i64`, JNI `jlong`, and Java
`long` carry the same value without a narrowing or signedness conversion.

Every result parser requires an explicit, valid positive `i32` ID and performs
an exact registry lookup. Missing, zero, negative, fractional, non-finite, or
out-of-range IDs are ignored as invalid transport. Settling the oldest pending
operation when a result has no ID is prohibited. Android success and failure
paths must echo the ID supplied by JavaScript; platform-internal method and JNI
signatures may change to carry it, but the public Android handler interfaces do
not.

An ID is correlation, not the Host dispatch boundary. Every callback created
by a runtime or Worker, including an ID-bearing result, also carries the
generation that created it through native ingress and the queued
`HostCommand`. `Host` compares that generation before invoking any JavaScript
hook; only a current command may proceed to exact ID lookup. A replacement
runtime's empty or non-matching pending map is never the first stale-callback
check.

Not every callback is naturally request-shaped. Singleton results, dialogs,
recorder notifications, and long-lived sensor, network, Bluetooth, keyboard,
camera, video, and audio streams must therefore satisfy one of these two rules:

- The result or resource has a Host-allocated ID that is validated before
  dispatch into JavaScript.
- The callback source captures the runtime generation that created it, and a
  platform-side gate drops it early when stale. The generation also travels
  through the platform's internal callback ABI, native ingress, and the queued
  `HostCommand`; Host dispatch compares it exactly with the current generation
  before calling JavaScript. The platform-side check is an optimization, not
  the authoritative correctness boundary.

Direct Host ingress that is not created by a runtime-owned manager, such as a C
host's soft-keyboard event, stamps the Host's current generation at enqueue.
If restart commits before that queued event is dispatched, the same Host-side
comparison rejects it as retired.

Unique IDs prevent stale ID-bearing callbacks from finding a new registry
entry, but do not by themselves cancel native work, protect an ID-less stream,
or release external resources. Restart therefore has a synchronous,
platform-neutral lifecycle fence with two phases. The read-only preflight
validates the current and candidate generations and the platform's ability to
perform its completion barriers without revoking callbacks or changing a
resource. Only a preflight failure may resume the old isolate. The subsequent
commit is irreversible: before the old isolate is dropped and before a
replacement isolate can be published, it advances the platform generation,
revokes all old-generation callback sources, and detaches all runtime-owned
resource registries. Once commit begins, any failure terminates the Host and
runs abort cleanup; the old runtime is never resumed after partial revocation.

Commit revokes retired callback sources, but correctness does not assume that a
Java token check and JNI enqueue are atomic. A callback that checked current
immediately before commit, or a retired command already queued during the race,
still carries its retired ID or generation and is rejected by Host dispatch.
Destruction or quiescence of an exclusive or power-sensitive resource,
including camera, recorder, location, sensors, Bluetooth discovery, BLE/GATT,
and their worker threads, must complete before commit returns and before
new-generation acquisition is admitted. When teardown must run on a platform
thread, the platform owns a completion barrier and the Host awaits it. Failure
or timeout of that barrier is a terminal restart failure; it is never converted
to successful revocation. Only non-exclusive work that cannot retain or
reacquire a native resource, such as an already-running image compression or
host-auth request, may finish physically after commit. Its late completion
remains ID- or generation-gated at Host dispatch.

Every resource-creating platform call carries the caller runtime's immutable
expected generation. The platform compares it atomically with the active
generation at the acquisition point; it never substitutes a newly loaded
current value. Retired Workers are stopped and joined before candidate
publication, and an old Worker racing with the transition is rejected even if
the replacement generation is already active.

A timeout may report a terminal diagnostic but cannot release an owned cleanup
barrier. Crossing the declared threshold permanently makes that restart
terminal: later cleanup may reach quiescence, but it cannot activate or publish
the candidate generation or convert the attempt to success. Cleanup failures
retain their concrete handles and the Session remains in a cleanup-only
terminal state until an explicit retry reaches quiescence. The Host thread
cannot exit while a platform, Worker, audio, or render cleanup barrier remains
incomplete.

Terminal Session close must not create a wait cycle with that rule. If the Host
is synchronously waiting inside a restart abort barrier, Android close first
drives the shared retained platform cleanup state to quiescence. Only after
that prerequisite succeeds may `NativeMethods.shutdown` request Host shutdown
and synchronously join the Host thread. A failed prerequisite leaves the
Session registered and returns a retryable cleanup error without entering the
native join. The prerequisite and the abort waiter reference the same cleanup
state; neither starts an independent fixed retry loop.

The restart fence invalidates runtime-owned dialogs and activity results,
location requests, media and recorder managers, cameras, videos, sensors,
network monitors, keyboard state, Bluetooth and BLE work, ads, pending
permission requests, audio contexts and players, offscreen canvases, and
JavaScript-owned render resources. It preserves Session-level registrations
such as error, message, auth, ad, permission, log, and subpackage handlers. It
also preserves standing host permission decisions; only a pending
JavaScript-owned permission request is retired. A new runtime must explicitly
create and start every resource it needs, and the platform rejects such
acquisition while the restart fence is in progress.

Audio and render threads use ordered reset barriers. The audio barrier clears
contexts, nodes, decode destinations, inner players, media players, and pending
events before new-generation commands are accepted. The render barrier removes
old JavaScript-owned canvas/WebGL resources and resets the surviving onscreen
surface to a fresh-runtime state without releasing the Session's compatible
native Surface lease. Shared image destruction remains part of this ordered
cleanup.

Restart publication is transactional. A read-only platform-preflight failure
leaves the old isolate published and resumes any subsystem paused before the
attempt. A commit failure, or any later failure after commit has begun to
retire the old generation, terminates the Host through the normal fatal-error
path; it must not install, resume, or dispatch callbacks into a partially
initialized replacement isolate. Before that termination completes, the
platform abort fence revokes and quiesces both partially retired old resources
and any resources the unpublished candidate created during prelude execution
or module evaluation. The Host publishes the new isolate and commits the new
generation only after runtime construction, prelude scripts, and module
evaluation have succeeded.

## 7. Native Component And Link Design

### 7.1 Content-Addressed V8 Inputs

Every platform build first verifies the V8 component manifest. It then
materializes the verified archive, bindings, and runtime DLLs under an immutable
directory whose name includes the component SHA-256 identity.

`RUSTY_V8_ARCHIVE` and `RUSTY_V8_SRC_BINDING_PATH` point into this immutable
directory. Replacing a V8 component changes the environment value observed by
Cargo and cannot reuse an rlib containing previous archive bytes.

Android, Linux, and Windows use the same materialization helper and schema.
Per-platform cache stamps and relying on a manual `cargo clean` are removed once
the content-addressed path is active.

Release packaging verifies that the V8 identity embedded in the linked package
matches the component selected by the package manifest.

### 7.2 Duplicate C++ Runtime Symbols

`--allow-multiple-definition` is removed from repository Cargo configuration and
all build scripts.

Android V8 and Skia are rebuilt with one compatible C++ runtime strategy so
their archives do not contribute conflicting global C++ runtime definitions to
the final link. The build fails on every duplicate strong global symbol.

The release gate records and audits the final link map. Any future duplicate is
a hard failure. Symbol localization or a linker flag that silently chooses one
definition is not an acceptable final design.

### 7.3 Clean Windows Build

Windows SDK construction runs entirely in a Windows-native build job. WSL may
remain a developer convenience, but it is not the release architecture.

Cargo builds first. A generated link manifest then captures the exact native
library search directories and libraries from Cargo output. The DLL link
consumes that manifest after the directories exist. The process must work with
empty Cargo and target directories.

The Windows package manifest covers `migo.dll`, `migo.lib`, V8 DLL/import
library, ANGLE DLLs, headers, CMake files, licenses, and build metadata. V8
patches and all shipped V8 bytes are represented in provenance.

### 7.4 Android AAR Entry Points

Shell and PowerShell AAR entry points use the same packaging implementation.
Neither script may invoke a release Gradle task without the verified package
index and artifact-manifest tool required by the Gradle gate.

`--skip-rust` is allowed only for non-release developer builds. Release builds
reject it. A release cannot infer current provenance from the presence of old
`.so` files.

## 8. CI And Release-Candidate Pipeline

### 8.1 Workflow Structure

Create one release-candidate workflow with these required jobs:

1. Source, formatting, lint, unit, adapter, ABI, and manifest gates.
2. Android AAR and C SDK build/test.
3. Linux SDK and Qt host-kit build/test.
4. Windows SDK build/test.
5. Android emulator behavior tests.
6. Physical Android device performance and power gate.
7. Cross-job release manifest assembly and verification.

The assembly job depends on every required job. No asset becomes a release
candidate before all dependencies succeed.

The workflow uploads CI artifacts only. A separate future tag-only publish job
may download the verified candidate, verify its source revision and manifest,
add signed provenance, and publish it. The current implementation does not
enable public publishing.

### 8.2 Fail-Closed Rules

All required workflows enforce:

- A required suite directory is absent: fail.
- A runner command exits non-zero: fail.
- A report is absent, stale, empty, malformed, or from another revision: fail.
- A required metric is absent or non-numeric: fail.
- Zero samples: fail.
- A requested release asset is absent: fail.
- A consumer build or first-frame marker is absent: fail.
- A test filter executes zero tests: fail.
- An unsupported skip condition occurs after a platform has a published
  baseline: fail.

Every targeted test wrapper asserts the expected number or identity of executed
tests when a filter is used.

### 8.3 Toolchain Pinning

Rust, Android NDK, cargo-ndk, Java, Gradle, Node, npm, CMake, Ninja, LLVM, MSVC
image, Linux sysroot, and packaging tools are pinned to explicit versions or
immutable digests.

GitHub Actions are pinned to immutable commit SHAs. Gradle distributions include
`distributionSha256Sum`.

Dependency installation is allowed during ordinary CI preparation. Release
packaging itself consumes lockfiles and verified component inputs and may not
silently upgrade a tool.

## 9. Performance Qualification

### 9.1 Benchmark Workloads

The Android release gate compares Migo and Android System WebView using the same
device, game content, session, screen state, and measurement harness.

At minimum, the benchmark suite contains:

- Canvas2D startup and steady animation.
- WebGL startup and steady animation.
- Asset-heavy content load.
- Mini-game adapter startup and input response.

### 9.2 Required Metrics

Store raw samples and report:

- Process start to content-ready, p50 and p95.
- Process start to first presented frame, p50 and p95.
- Frame time p50, p95, and p99.
- Missed-frame or jank rate.
- Steady PSS and RSS after the defined stabilization period.
- Average and peak CPU during the defined workload.
- Energy or battery drain over the defined measurement window.
- Installed and compressed package size.

Reports include device model, OS build, WebView version, GPU, thermal state,
screen refresh rate, Migo revision, product profile, workload revision,
cold/warm policy, run count, and timestamps.

### 9.3 Gate Semantics

Baseline thresholds are versioned and reviewed. Missing metrics are failures,
not skips. Performance regressions beyond the approved threshold fail the
release candidate. Warning-only thresholds may exist for PR feedback, but the
release gate has no warning-only required metric.

README performance tables are generated from an immutable summary checked into
the benchmark-results location and linked to raw reports. Marketing copy does
not contain an unqualified "faster" or "lower memory" claim without those
results.

## 10. Examples And Consumer Experience

### 10.1 Resolver

The resolver:

- Uses the unified product version.
- Constrains exact Migo product and platform identity.
- Downloads into a temporary directory.
- Validates signed provenance or a trusted pinned digest.
- Validates archive structure before extraction.
- Verifies the embedded package manifest.
- Atomically replaces the installed SDK only after full verification.
- Writes an installed-version receipt and invalidates a cache when the pin
  changes.
- Distinguishes not-found, transport, authentication, integrity, and local
  configuration errors.

Once a platform release exists, transport failure, missing assets, and missing
provenance are hard CI failures.

### 10.2 Android Examples

- Both launcher paths load the deployed `demo` game by default.
- Surface recreation reattaches the existing Session instead of leaking it.
- Every snippet closes exactly one Session.
- The app and snippets build against the release-candidate AAR.
- An emulator test launches both integration paths and requires content-ready
  and first-frame evidence.

### 10.3 Linux And Windows Examples

- Touch points set `CHANGED` and `REMOVED` according to the public contract.
- Input send results are handled; terminal transitions are not ignored.
- Nested directories and dotfiles are copied safely.
- Resize and DPI changes update the Migo Surface.
- Teardown waits for the documented release and Engine shutdown guarantees.
- Linux and Windows examples build from clean resolved SDKs in their native CI
  jobs and produce first-frame evidence.

### 10.4 Authentication Relay

The development relay binds to loopback by default. Remote-device mode requires
an explicit listen address and a randomly generated bearer token.

All auth request, poll, and response endpoints authenticate the token. Empty
game IDs are rejected. Request bodies, queue length, waiting clients, and
request lifetime are bounded. CORS is disabled by default and, when explicitly
enabled, uses a configured origin rather than `*`.

The relay is documented as a development-only component and is not a production
authentication architecture.

### 10.5 Windows Runner Input Safety

Game IDs and durations are validated before generating a batch file. Generated
batch arguments are quoted and escaped for `cmd.exe`, or are passed through a
non-shell process interface. User-controlled data is never interpolated as
batch syntax.

## 11. Public Documentation

### 11.1 Root README

The first viewport contains:

- Product category and WebView-alternative position.
- Source-available license badge and wording.
- Supported platform and architecture summary.
- Links to one verified performance table and one real example.

The next sections contain:

1. Measured advantages with reproducible methodology.
2. Android, Linux, and Windows artifact and minimum-version matrix.
3. Five-minute consumer integration for Gradle and CMake.
4. Compatibility matrix for Canvas2D, WebGL/WebGL2, adapter BOM/DOM, mini-game
   `wx` APIs, and verified engine versions.
5. Security and sandbox boundaries.
6. Build-from-source and contributor links.
7. BSL production-use summary with a link to authoritative legal text.

### 11.2 Compatibility Claims

Claims such as "runs unmodified" require a versioned test entry naming the
engine version, workload, platform, and result. Untested engines and APIs are
listed as planned or partial.

The matrix distinguishes:

- Implemented by the native runtime.
- Implemented by the adapter.
- Host-provided capability.
- Unsupported browser feature.

### 11.3 Build And Contributor Documentation

`BUILD.md`, `CONTRIBUTING.md`, platform READMEs, and `CHANGELOG.md` are generated
or manually updated against the same release matrix. They must not:

- Describe Android as the only product.
- Name stale artifact paths.
- Recommend a release command that the build rejects.
- Recommend skipping a failing required test.
- Claim a toolchain version different from the pinned one.
- Describe a released platform as a spike.

## 12. Test Strategy

Implementation follows test-driven development for each defect:

1. Add a test reproducing the current failure.
2. Run it and observe the expected failure.
3. Implement the smallest architectural correction.
4. Run the focused test.
5. Run the affected platform or shared regression suite.
6. Commit the verified change.

Required test layers are:

- Pure Rust unit and concurrency tests.
- C ABI layout, validation, and lifecycle tests.
- Java/Kotlin unit and compile-contract tests.
- JavaScript adapter tests.
- Package-manifest and reproducibility tests.
- External CMake, pkg-config, Gradle, and DLL consumers.
- Platform-native integration tests.
- Android emulator behavior tests.
- Physical Android device performance tests.

Text-grep contract tests may supplement behavioral tests. They may not be the
only proof of lifecycle, linking, rendering, or packaging behavior.

## 13. Delivery Phases

### Phase A: Correctness Foundation

- Fix thread ownership and Engine shutdown joins.
- Enforce Surface `PlatformIdentity`.
- Remove the X11 threading precondition through a Migo-owned connection.
- Implement bounded lossless terminal input state.
- Correct pointer semantics.

Exit criterion: lifecycle and saturation stress tests pass on host-testable
paths, and public contracts describe the verified behavior.

### Phase B: Hermetic Cross-Platform Builds

- Add the unified version source.
- Add content-addressed V8 materialization.
- Remove duplicate-symbol linker relaxation.
- Make release metadata deterministic.
- Repair Android PowerShell packaging.
- Replace the Windows warm-target link flow.
- Add complete three-platform manifests, licenses, notices, and SBOM.

Exit criterion: clean local builds produce contract-valid packages and a second
build reproduces the first build's bytes.

### Phase C: Native Platform Qualification

- Validate Android AAR/C SDK and emulator behavior.
- Validate Linux shared/static SDK, X11, Wayland, and Qt.
- Validate Windows DLL/CMake/ANGLE/Win32 behavior.

Exit criterion: each supported platform reaches first frame, handles resize and
input, and exits without live Migo threads from a clean consumer.

### Phase D: Examples, Security, And Developer Experience

- Correct and test every supported example.
- Harden resolver, authentication relay, and Windows command generation.
- Add native CI coverage for all examples.

Exit criterion: a clean examples checkout resolves the candidate SDK and reaches
first frame on all three platforms.

### Phase E: Performance And Release Candidate

- Replace fail-open device and profile gates.
- Record WebView comparison data.
- Assemble and verify the full artifact matrix.
- Rewrite product, build, platform, contributor, and changelog documentation.

Exit criterion: the release-candidate workflow can assemble every required
artifact only after all correctness, compatibility, security, and performance
gates succeed.

## 14. Definition Of Done

Migo is ready for external delivery only when all statements below are true:

- Android, Linux, and Windows build from clean documented environments.
- Every required artifact in Section 5.2 exists and verifies.
- Repeated equivalent builds are byte-for-byte reproducible.
- No release build uses `--allow-multiple-definition`, `--skip-rust`, stale
  native bytes, wall-clock metadata, or warm-cache discovery.
- Engine destruction leaves no thread executing Migo code.
- Surface release permits immediate release of the documented host resource.
- Reattachment either works within the stated compatibility class or fails
  synchronously.
- Terminal input state converges under saturation.
- Android Full and Slim lint pass without a lint baseline; protected platform
  calls have manifest declarations appropriate to the product profile and
  explicit runtime permission/revocation handling.
- Required CI gates fail on missing tests, reports, samples, and assets.
- Android, Linux, and Windows examples reach a verified first frame.
- Physical Android performance results support every README performance claim.
- Packages carry licenses, notices, SBOM, manifests, checksums, and trusted
  provenance hooks.
- README, build docs, platform docs, changelog, and examples describe the same
  version, support matrix, and integration flow.
- Both repositories are clean after verification.
- All implementation commits remain local until the user explicitly chooses to
  push them.

## 15. Explicit Non-Goals

- Publishing a public GitHub Release.
- Pushing commits or tags.
- iOS, macOS, OpenHarmony, WinUI, or non-x86_64 desktop delivery.
- Full browser DOM, CSS, HTML layout, browser extensions, or Chromium API
  compatibility.
- Productionizing the development auth relay.
- Changing BSL 1.1 to an OSI license.
