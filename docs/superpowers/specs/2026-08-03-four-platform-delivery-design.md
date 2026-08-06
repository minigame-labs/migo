## Migo Four-Platform Delivery Design

**Status:** Awaiting user review

**Date:** 2026-08-03

**Relationship to the existing design:** This document supersedes Section 3
(Required Delivery Matrix) and Section 15 (Explicit Non-Goals) of
`docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md`. Every
other section of that document — engineering principles, release identity,
runtime correctness, native component and link design, CI fail-closed rules,
performance qualification semantics, examples, documentation, and test strategy
— remains authoritative and is extended rather than replaced. Where this
document adds a requirement to an inherited section, the inherited requirement
still applies to all four platforms.

**Repositories:** `migo` (runtime, platform integrations, packaging, release
automation, product documentation) plus five helper repositories whose delivery
roles Section 9 makes binding: `migo-examples`, `migo-bench`,
`migo-conformance`, `migo-test-suite`, `migo-android-demo`.

## 1. Objective

Make Migo deliverable on Android, Linux, Windows, and HarmonyOS. A
delivery-ready revision must be correct under normal lifecycle and failure
conditions, reproducible from clean builders, integrable with effort comparable
across all four platforms, performance-measured against a real per-platform
baseline rather than performance-claimed, consumable through tested examples,
and documented as the product that is actually shipped.

Correctness precedes performance, and performance is a gated requirement rather
than an aspiration: every performance claim in this document has a named metric,
a recorded baseline, and a failing condition.

The work ends at a locally verified release candidate. This effort does not
push, tag, or publish.

## 2. Platform Matrix

### 2.1 Android

Unchanged from the inherited Section 3.1: minimum API 26, `arm64-v8a` and
`x86_64`, Full and Slim AARs plus per-ABI C ABI SDKs, and the full permission
contract with its API 26/28/31 merged-manifest matrix.

### 2.2 Linux

Unchanged from the inherited Section 3.2: glibc x86_64 at the declared floor,
X11 and Wayland with equal lifecycle guarantees, versioned shared library,
static library, headers, CMake package, pkg-config package, and Qt 6 host kit.
Section 3 of this document adds the cross-platform desktop host kit.

### 2.3 Windows

Unchanged from the inherited Section 3.3: Windows 10 and 11 x86_64, MSVC x64,
Win32 `HWND`, ANGLE with packaged runtime DLLs, `migo.dll`, `migo.lib`,
headers, and CMake package. Section 3 of this document adds the cross-platform
desktop host kit and the Qt 6 kit on Windows.

### 2.4 HarmonyOS

HarmonyOS coverage is delivered through two distinct paths, and documentation
must never conflate them:

- **HarmonyOS 4.x and earlier** are AOSP-based and install Android packages.
  These devices are served by the existing Android `arm64-v8a` product. No
  HarmonyOS-native artifact is required for them. A claim of support for this
  tier requires an install-and-run verification on a real HarmonyOS 4.x device,
  recorded like any other device evidence.
- **HarmonyOS NEXT and later** are pure HarmonyOS, do not install Android
  packages, and require the HarmonyOS-native product defined below.

Delivered target: HarmonyOS NEXT. OpenHarmony distributions are documented as
planned, not delivered, until they have their own SDK build, symbol-floor audit,
and device evidence.

- Architectures: `arm64-v8a` for devices; `x86_64` for the emulator and
  development only. The `x86_64` artifact is labelled as an emulator/development
  artifact in the manifest, the archive name, and the documentation. It never
  substitutes for `arm64-v8a` device evidence.
- Products:
  - C ABI SDK for `arm64-v8a`.
  - C ABI SDK for `x86_64`.
  - One reusable ArkTS package (HAR) exposing the Tier 1 facade defined in
    Section 3.
- API floor: the lowest HarmonyOS NEXT API level at which every platform symbol
  Migo references resolves. API 12 is the intended floor. The floor is a proven
  value, not a declared one: `scripts/test-ohos-symbol-floor.sh` must run with
  both `MIGO_OHOS_FLOOR_SYSROOT` set to the floor-level sysroot and
  `MIGO_OHOS_NEWER_SYSROOT` set to a later sysroot, so that any symbol
  introduced after the floor is detected rather than assumed absent. If the
  audit proves a symbol requires a later level, the floor rises to the lowest
  provable level, the blocking symbol and its owning capability are recorded,
  and `compatibleSdkVersion` matches the proven floor exactly.
- Product profile: HarmonyOS ships the Slim capability set, defined mechanically
  by the product profile flags rather than by enumeration — `MIGO_API_SENSORS`,
  `MIGO_API_MEDIA`, and `MIGO_API_CONNECTIVITY` are off, and every capability
  outside those three is on. Audio is not one of those flags: it is a core
  capability present in both Android profiles, so HarmonyOS audio playback is in
  scope and requires OHAudio integration. The delivered HarmonyOS capability set
  therefore covers what the large majority of games actually use — rendering,
  audio playback, touch and pointer input, storage, networking, and timers.
  Sensors, camera, video, audio recording, and BLE are documented as planned for
  HarmonyOS. Shipping a HarmonyOS package whose declared permissions exceed its
  honoured capabilities is prohibited.
- Permission model: the HAP and HAR module declarations list exactly the
  permissions the shipped capabilities require, and no others. The inherited
  rule that Migo never requests dangerous permissions implicitly, that every
  protected operation rechecks the current grant before entering the framework,
  and that denial or revocation fails through the documented error path without
  leaking a resource or publishing a false success state applies to HarmonyOS
  without modification.
- Required validation:
  - Clean SDK build for both architectures through
    `scripts/build-ohos-sdk.sh`.
  - `scripts/test-ohos-sdk-contract.sh`, which verifies the built artifact,
    links a real C consumer with the SDK toolchain, and cross-checks manifest
    capability claims against the artifact's undefined symbols.
  - `scripts/test-ohos-symbol-floor.sh` in the two-sysroot configuration above.
  - `scripts/test-ohos-toolchain-contract.sh`, proving every recorded compiler
    came from the OHOS SDK and the ABI is musl rather than bionic.
  - HAP and HAR consumer builds.
  - Emulator attach, content-ready, first-frame, resize, background/foreground,
    surface recreation, multi-touch input, audio playback, detach, and shutdown.
  - `arm64-v8a` device qualification for the same behaviour set.
  - Device performance and power measurements against the ArkWeb baseline.

## 3. Integration Contract

Integration effort must be comparable on all four platforms. API shape must not
be uniform: forcing a Java-shaped API onto C++ or ArkTS produces a worse
integration, not a better one. The invariant is that on every platform a host
reaches first frame by creating a session from a content identity, handing over
a native window, and forwarding lifecycle and input, without implementing engine
internals.

Measured starting point: the C ABI is roughly fifteen functions across
`include/migo/session.h` and `include/migo/surface.h`, yet the Linux and Windows
examples need about 235 and 236 effective lines to reach first frame. That cost
is window-system plumbing — window creation, event loop, input translation — not
API complexity. The remedy is host kits that own the plumbing, which is
precisely what `MigoGameActivity` does for Android and what
`platforms/linux/host-kit/.../qt6` does for Qt.

### 3.1 Tiers

- **Tier 0 — C ABI.** Present on all four platforms and the sole authority for
  runtime semantics. Every lifecycle rule, every error code, and every ownership
  barrier is defined here.
- **Tier 1 — language-native facade** for platforms whose application model is
  opinionated about lifecycle, surfaces, and permissions: the Android Java/Kotlin
  AAR (existing) and the HarmonyOS ArkTS HAR (new).
- **Tier 2 — desktop host kits** that own window, event loop, and input
  plumbing:
  - The Qt 6 kit becomes cross-platform. It is currently bound to X11 under
    `platforms/linux/host-kit`; it must support Linux X11, Linux Wayland, and
    Windows, and move to a platform-neutral location.
  - A header-only C++ RAII wrapper with no GUI-toolkit dependency, usable from
    Win32, SDL, GLFW, and in-house engine hosts. It provides session
    construction from a content identity, surface attach and rebuild, input
    forwarding, and deterministic teardown that honours the documented release
    barriers.

### 3.2 Facade Rules

1. A facade is a thin adapter. It must not hold a state machine that the C ABI
   does not have, and must not define an error taxonomy of its own.
2. Every facade lifecycle path maps to a documented C ABI call sequence, and a
   contract test pins that mapping. A behaviour reachable through a facade but
   not expressible through Tier 0 is a defect in the facade.
3. Teardown through a facade must satisfy the same release and shutdown barriers
   as Tier 0. A facade may not report a resource released earlier than Tier 0
   would.
4. A facade never silently degrades. When the underlying operation fails, the
   facade surfaces the Tier 0 error identity.

### 3.3 Integration Parity Gate

Each supported integration path in `migo-examples` delimits a single integration
region with explicit begin/end markers. CI measures, inside that region only,
the count of non-blank non-comment lines and the count of distinct Migo API
calls, for every platform and every tier. Every platform carries a Tier 0
example, because the Tier 0 line count is the denominator the facade tiers are
judged against.

The measurements are committed as a baseline. The gate fails when:

- any measurement regresses against the committed baseline, or
- a Tier 1 or Tier 2 **line count** exceeds one quarter of the Tier 0 line count
  for the same platform, because a facade that does not deliver a fourfold
  reduction in integration code does not justify the delivery surface it adds, or
- a required integration path has no measurement.

The API-call count is reported and must not regress, but is not ratio-bounded: a
facade may legitimately make the same number of logical calls while removing the
plumbing around them.

Tier 0 paths are measured and reported but not bounded; the raw C ABI is the
escape hatch and is allowed to be verbose.

### 3.4 Canonical Host Callback Set

Integration has two directions. Section 3.3 governs the outbound direction — what
the host calls to reach first frame. This section governs the inbound direction —
what the host implements so Migo can call it. The two are independent, and a host
that reaches first frame but cannot implement the callbacks its content needs is
not integrated.

The current inbound surfaces are not equivalent, they are nearly complementary.
Android's Java facade exposes product-level handlers — message, auth, permission
with its sink, ad with its sink, game log, and subpackage — and handles the
runtime-level concerns internally. The C ABI exposes runtime-level callbacks in
`MigoHostCallbacks` — dispatch, ready, error, exit requested, surface lost,
surface released, request frame, and the three keyboard callbacks — and has no
product-level handler at all. HarmonyOS exposes none in either category; its NAPI
bridge exports only `start`.

One canonical host callback set is therefore defined at Tier 0 and exposed on
every platform:

- Runtime-level: thread dispatch, ready, error, exit requested, surface lost,
  surface released, frame request, and soft-keyboard show/hide/update.
- Product-level: host-to-content messaging, authentication, permission,
  advertising, game log, and subpackage loading.

Rules:

1. **Interface parity, not business parity.** Every platform exposes the whole
   canonical set in its idiomatic form. What is equivalent across platforms is the
   interface and the meaning of leaving it unregistered — never an obligation on
   the host to implement a business capability it does not have.
2. **Registration is per capability and optional.** A host pays only for the
   capabilities it uses.
3. **The unregistered path is specified and tested.** For every product-level
   capability, a content request that arrives with no registered handler settles
   through its documented error path. It never hangs, never returns a false
   success, and never leaves the request pending. This requirement already exists
   for advertising in the inherited ledger and is generalised here to the whole
   product-level set.
4. **Asynchronous capabilities are asynchronous everywhere.** Permission and
   advertising are defined as a handler plus a result sink, so a host may resolve
   them later — for example from a desktop dialog. A host that can answer
   immediately resolves the sink synchronously, which is a special case rather
   than a separate shape. Defining these as synchronous because a particular
   platform happens to answer from configuration is prohibited.
5. **Platform forms.** In the C ABI each capability has its own versioned
   registration structure carrying `struct_size`, `abi_version`, its function
   pointers, and its `user_data`, installed through its own entry point. The
   existing `MigoHostCallbacks` layout and its static assertions are not
   disturbed, and a host is not forced to restate unrelated capabilities. The
   desktop C++ kit surfaces the same capabilities as an interface the host
   implements. HarmonyOS surfaces them as ArkTS interfaces across the NAPI
   bridge, which is entirely new work. Android keeps its existing Java handler
   interfaces.
6. **Facades add no capability and hide none.** A capability reachable on one
   platform and absent on another is a defect, not a platform difference, unless
   Section 15 lists it as a non-goal.

Each capability on each platform has a contract test proving the callback is
reachable from real content and that its documented semantics hold, including the
unregistered path. Compiling against the declaration is not evidence.

The parity gate in Section 3.3 additionally fails when a platform is missing a
canonical callback, when a registered capability has no reachability test, or
when an unregistered capability has no settlement test.

## 4. Release Identity

One machine-readable product version source under `release/` governs all four
platforms. The release-candidate value is `0.10.0-rc.1`. Gradle metadata, CMake
package versions, the HAR version, archive filenames, example pins, generated
version JSON, and workflow assertions all derive from it.

The per-platform release trains currently used by `migo-examples`
(`v0.9.0`, `linux-sdk-0.1.0`, `windows-sdk-0.1.1`) are retired. Example pins
collapse to a single version file. Archive filenames retain a platform and
architecture suffix so a single version can still be distributed per platform.

`MIGO_ABI_VERSION_CURRENT` remains the numeric C ABI negotiation version and is
never used as a package version.

## 5. Artifact Matrix

One release candidate contains all of these required files:

- `migo-0.10.0-rc.1-android-full.aar`
- `migo-0.10.0-rc.1-android-slim.aar`
- `migo-0.10.0-rc.1-android-c-arm64-v8a.tar.gz`
- `migo-0.10.0-rc.1-android-c-x86_64.tar.gz`
- `migo-0.10.0-rc.1-linux-x86_64.tar.gz`
- `migo-0.10.0-rc.1-windows-x86_64.zip`
- `migo-0.10.0-rc.1-ohos-arm64-v8a.tar.gz`
- `migo-0.10.0-rc.1-ohos-x86_64-emulator.tar.gz`
- `migo-0.10.0-rc.1-ohos.har`
- `SHA256SUMS.txt`
- `release-manifest.json`
- `sbom.spdx.json`
- per-platform test summaries
- per-platform benchmark summaries
- the integration parity measurement report

Each platform archive carries public headers and libraries, package-manager
metadata, the desktop host kit headers where applicable, `LICENSE`, `LEGAL.md`,
`NOTICE`, complete applicable third-party notices, an embedded package manifest
covering every regular file, and build metadata naming the exact source revision
and toolchain identities.

## 6. Correctness Foundation

All inherited Phase A requirements (A1 through A13) apply and must close before
the facades in Section 3 are built. A facade layered over unverified semantics
only makes the underlying defect harder to diagnose.

The following are additionally required.

### 6.1 Outstanding Permission-Path Debt

The counted permission lease introduced for the Android BLE admission path has
two recorded residual risks that must close, because both are performance
requirements as well as design requirements:

- Cancellation actions must not execute while the per-session permission monitor
  is held. Snapshot and mark cancellations under the monitor, run them after
  releasing it while retaining the transition lock, then reacquire to remove
  successes and retain failures for retry.
- No per-event path may take a lock shared across sessions, and no per-event
  path may allocate. The permission gate's session lookup currently takes a
  monitor on a map shared by every session on each BLE notification, and each
  callback allocates a connection wrapper plus capturing lambdas. Section 8
  defines the regression tests that make this a gate rather than a note.

### 6.2 HarmonyOS Correctness

- The ArkTS-to-native bridge exposes a complete lifecycle. Today only
  `start(filesDir, cacheDir, contentId)` is exposed; there is no stop, resize,
  or teardown entry point, and `EntryAbility`'s foreground and background hooks
  do not reach the engine. Start, stop, resize, surface recreation, foreground,
  and background must all be expressible and must map to Tier 0 sequences.
- The ArkTS bridge also carries the canonical host callback set from Section 3.4
  in both directions. HarmonyOS currently has no inbound callback path at all, so
  every runtime-level and product-level callback is new work here.
- `OHNativeWindow` ownership follows the same discipline as the Migo-owned X11
  connection: Migo releases only the reference it acquired, the host keeps its
  own, and the release barrier is published only after no Migo thread can touch
  the window again.
- HarmonyOS gains a `PlatformIdentity` entry so surface reattachment is accepted
  only within a compatible platform class and rejected synchronously otherwise.
  The inherited Section 6.2 has no HarmonyOS row today.
- Multi-touch is verified. Only single-pointer input has been exercised, and the
  touch translation contains emulator-specific handling that the multi-finger
  path does not cover.
- The restart generation boundary, thread ownership, and shutdown barriers apply
  to HarmonyOS exactly as to the other platforms.

### 6.3 Desktop Host Kit Correctness

The cross-platform Qt 6 kit and the header-only wrapper are each covered by a
compile contract and a lifecycle contract that prove attach, resize, input
forwarding, detach, and teardown ordering on both Linux window systems and
Windows.

### 6.4 Concurrent Session Isolation

Migo supports running several games concurrently in one process. One Engine may
own many Sessions, and each Session is isolated from the others. This is a
product guarantee, so it is gated rather than assumed.

The public headers currently permit multiple Sessions only by the absence of a
prohibition. They must state the guarantee affirmatively: how many Sessions an
Engine may own, whether a process may create more than one Engine, and that two
Sessions may be driven concurrently from two host threads while calls through any
single Session remain serialised by the host.

**Properties already enforced, which become gated requirements:** one V8 isolate
per Session, with a fresh isolate on restart and no `SharedArrayBuffer` store
crossing sessions; per-game filesystem, key-value, and quota isolation derived
from the game identity; per-host fairness on the shared IO executor — **now gated
against the real executor rather than read, see Section 7.3**; an Engine that
refuses destruction while any Session is live; per-session platform manager
registries; and per-session permission monitors.

**Defects that must be fixed before multi-game support is claimed:**

1. The process-global text texture cache is keyed without session identity while
   its entries hold raw GL texture names owned by one Session's context. Two
   games rendering identical text at identical size, weight, colour, and canvas
   dimensions collide on one key, so one session can receive a texture name that
   is meaningless in its own context, or delete a name the other still uses. The
   font generation counter is likewise process-wide, so one game reloading a font
   invalidates every other game's cached text.
2. Device-exclusive resources have no arbitration. Camera, microphone, the
   Bluetooth adapter, and audio focus are single system objects, yet each Session
   holds its own manager and acquires them independently. The second acquirer
   silently breaks the first, and the failure reaches content as an opaque error.
   Each such resource needs explicit ownership with a documented outcome for the
   losing Session — never a silent takeover.
3. The Rust permission gate's `open` rejects any host id at or below a
   process-wide high-water mark, and its return value is discarded at the call
   site. Two Sessions starting concurrently can reach `open` in the opposite
   order to their id allocation, after which the losing Session has no host
   control and every permission check returns denied for scopes the user actually
   granted. This is a silent, ordering-dependent, permanent failure and must be
   made either order-independent or fail-closed at Session creation.
4. Shared budgets let one game degrade another: the image cache pool, the Skia
   resource budget, the on-disk code cache, and the single worker serving all
   audio streaming. Each needs either per-session accounting or a documented and
   tested degradation policy.

   **Fixed under task 0.19, and no two of the four the same way** — which is Section
   6.5's tiers deciding it rather than a uniform rule. The image cache kept its sharing
   and gained per-entry ownership; the Skia budget's denominator moved to the process
   scope its numerator already had, and its memory-pressure ceiling became transient
   rather than a stored budget one game's warning left behind for every other game; the
   code cache kept its sharing and its budget moved to the directory holding the bytes;
   the audio streaming worker gave its CPU-bound decode to a blocking thread, which is
   what the new gate in Section 7.3 exists to hold it to.
5. The Engine configuration takes the file, cache, and code-cache roots per
   Engine rather than per Session, so a single-Engine host cannot give two games
   different roots. Isolation below the root is by game identity; the shared root
   must be documented, or the roots must move to Session scope.

   **Resolved under task 0.20 by taking the first option, and the three roots do
   not share one reason.** `files_dir` and `cache_dir` are the *host
   application's* directories, granted to it once by the platform — one
   `getFilesDir()` per Android app — so there is no second one for a second game
   to be given, and Session scope would add a way to configure isolation wrongly
   rather than a way to obtain it: a host can already hand two Sessions one root,
   and would then also have to be trusted not to hand them one content id.
   `code_cache_dir` is the root that moving would actively damage, because
   Section 6.5 *requires* it to be shared and defect 4's fix put its budget on the
   directory precisely because the directory is one; per-Session code caches would
   give each Session its own copy of every compile.

   The obligation this leaves on the host is now stated where the roots are
   declared rather than implied: **content ids must be distinct between
   concurrently live Sessions**, since every per-game path is derived from that id
   and nothing in the engine enforces uniqueness. Refusing a duplicate is
   deliberately *not* done — two Sessions of one title is a legitimate thing for a
   host to want, and the engine cannot tell that apart from a mistake.

**Test requirement.** No test anywhere currently creates two concurrent Sessions.
Every isolation property above is correct by reading rather than by execution.
Each enforced property needs a behavioural test that starts two Sessions, loads
two different games, and asserts non-interference, and each defect above needs a
regression test that fails before its fix. A property that has never been
executed concurrently is unverified and may not be claimed in documentation.

### 6.5 What Sessions Share, And What They Must Not

Concurrent games make every process-wide cache a decision, and the answer is not
uniformly "make it per-session". Splitting a cache that holds
context-independent bytes duplicates memory for no benefit, which is the opposite
of what running several games in one process is for. Three tiers, decided by what
the entries actually hold:

**Must be per-session — the entry names a resource owned by one Session.** A GL
texture name is meaningful only inside the EGL context that minted it, and each
Session builds its own. The text texture atlas is here, which is why Section 6.4
defect 1 required splitting it rather than merely re-keying it.

**Should stay shared — the entry is context-independent bytes.** Decoded image
pixels, font file bytes, and the on-disk V8 code cache are the same bytes whichever
Session asked for them. Two games loading the same asset should hold one copy.
Sharing is only sound when the key carries the resource's *real* identity. A key
that were merely the virtual path would hand one game another game's pixels, and
that is the property to check before sharing anything else.

**The decoded-image key passes that test for directory-backed mounts and fails it
for pack-backed ones.** This section previously asserted it passed outright, which
was checked only against the directory case. When a `/code` path resolves to a real
file, the token hashes the resolved real path together with the file's size, mtime
and mount origin, so two games whose virtual `/code/logo.png` are different files do
not collide. When it resolves inside a package there is no real path, and the key
becomes `(virtual_path, source_mounted_at)` — where `source_mounted_at` counts
mounts within **one** `MountTable`, of which every Session has its own, and a base
mount is `1` in all of them. Two games that ship different packages therefore
produced an identical key for the same virtual path and the second was served the
first's pixels — the production case, since a shipped game is normally pack-backed,
rather than an edge one.

**Fixed under task 0.28.** A resolution now carries a `source_identity` that means
the same thing in every Session: the package's own identity where the backend has
one, so two Sessions mounting byte-identical packages still share one decoded copy,
and an id unique to that mount within the process otherwise, so a backend that
offers no identity loses the sharing rather than colliding. The requirement this
section states is therefore now met by construction for both mount kinds, and the
lesson stands: check what the key actually carries before sharing anything, and check
it for every backend the key can come from, not just the one in front of you.

**A game's package is untrusted with respect to every other game's, and the
property a shared key must therefore carry is that producing it requires holding
the content.** This was previously left open. It has to be stated this way because
a game installs its own subpackages and so chooses their bytes, their mount prefix
and their entry paths, and because nothing registers a package signature verifier
today — `verify_package_signature` accepts after one warning when none is present,
so "a crafted package cannot be installed" is not available as a reason to key on
anything weaker.

With that property, sharing a decoded entry cannot disclose anything: two mounts
agree on a key only when their packages are byte-identical, and a Session that can
produce the key already holds the bytes. Entry metadata cannot carry it, because
the format's per-entry integrity primitive is a CRC32 and a package can be built to
match another's per-entry paths, sizes and CRC32s at equal length — cheaply, since
appending a message's own CRC32 drives the result's CRC to a constant. So the
identity is a SHA-256 over the package file's bytes, truncated to the width of the
key space it feeds.

**What that costs, and where the cost is paid.** Nowhere on a session start and
nowhere per read: an install already holds the whole package in memory to hand to
the signature verifier, so the digest is taken there and recorded in the install
manifest, and a restore takes the recorded value and reads only the package index.
A record predating the field is digested once and written back. Two consequences
are accepted deliberately: content packed twice with different chunk sizes no
longer shares, which loses sharing rather than safety; and a 64-bit key space means
a second preimage costs on the order of 2^64 hashes for another game's decoded
pixels — the width is the key space's, not the digest's, and `ResolvedCode` plus
the on-disk derived-asset key would truncate a wider value at the next hop.

**This requires the install record to be out of the game's reach, and it was not.**
`/cache` mapped read-write onto the per-game cache root, which holds the install
store, the manifest and install staging directories. A recorded digest there would
have been a label the game picks — the defect task 0.28 removed when it stopped
keying on `name` and `version` — and a staged package could have been substituted
between the moment an install validated and digested it and the moment it was
renamed into place. `/cache` now maps to a dedicated subdirectory, so every VFS
root is a directory of its own and runtime state is a sibling of all of them. The
same containment is what makes restore's other use of the record — the mount prefix
it mounts without validating — safe.

**Shareable in principle, unmeasured today.** Skia `Typeface` objects are CPU-side
and parsed per render thread, so N games parsing the same font parse it N times.
Whether that is worth sharing is a measurement, not an assumption, and the GPU glyph
atlas built from them stays per-session regardless.

What a shared cache owes in exchange for being shared:

- **Per-session reference accounting.** A Session ending drops its own references
  and nothing else. Clearing a shared cache on one Session's teardown is a defect,
  not hygiene.
- **Fair eviction.** One game's working set may not be evicted because another game
  is loading heavily. A shared byte budget with no per-session accounting lets one
  Session starve another.
- **A key that carries real identity**, as above.

**The on-disk code cache is shared, and takes the second of the two options Section
6.4 defect 4 offers.** Its budget is now owned by the directory rather than by each
Session's handle, which is what the ceiling means: it was enforced per instance while
the bytes it counted lived in a directory `MigoEngineConfig` scopes to the Engine, so
N Sessions admitted N times the ceiling, one Session's eviction deleted files another
Session's counter still claimed, and two Sessions could write and read one path at
once. What it does **not** have is per-session fair eviction: eviction is LRU by mtime
across the whole directory, so a game compiling heavily can evict another game's
entries. That is the documented degradation rather than an oversight — a shared entry
is exactly what two games loading the same module should get, attribution would have to
live on disk to survive a restart, and the cost of losing an entry is one recompile.
The gate covers the budget, not the fairness.

**Its Engine scope is load-bearing, which task 0.20 settled rather than assumed.**
"One budget per directory" is only a budget while the directory is one, so the
question defect 5 raised — whether the roots belong on the Session instead — is
answered here for this root and answered *differently* from the other two: moving
`code_cache_dir` to Session scope would silently retract the sharing this section
requires, not merely relocate a path. Both the header and Section 6.4 defect 5 now
say so where a host and a maintainer respectively will read it.

### 6.6 What A Game May Name

A game may name a path only inside its own sandbox, and every op that takes one
resolves it: `resolve_path_vfs` accepts `/code`, `/user`, `/cache` and `/tmp` and
refuses any other absolute path, and the audio and image resolvers refuse the same.
This section exists because one op did not, and the exception is instructive rather
than incidental.

`op_install_subpackage` took a real filesystem path from JS and handed it to the zip
ingest unresolved. The value is produced by the host, which is trusted, but it
reaches the runtime *through the game's own JS* — so the runtime could not tell the
host's path from one the game invented. Any zip the app process can read, the app's
own package among them, could therefore be ingested and read back through the game's
`/code`.

**A host-produced path must not travel through content to reach the runtime.** It is
held where the runtime received it and the game names the request it belongs to;
`intercept_download_result` strips it from the download result before the payload
reaches JS, and the install takes it back by request id, keyed per session so one
game's request number cannot name another's download. This is the general rule for
host-produced file references, not a fix for one op: a validated path would still be
a path the game chose, and validation is the weaker instrument — it constrains where
the file may be rather than establishing who named it.

## 7. Performance Qualification

### 7.1 Per-Platform Baseline

Each platform is measured against the system web runtime it is positioned
against, using the same content, session, screen state, and harness:

| Platform | Baseline |
| --- | --- |
| Android | Android System WebView |
| HarmonyOS NEXT | ArkWeb |
| Linux | Chromium |
| Windows | WebView2 |

### 7.2 Required Metrics

Raw samples are stored and these are reported per platform: process start to
content-ready p50/p95; process start to first presented frame p50/p95; frame
time p50/p95/p99; missed-frame or jank rate; steady resident memory after the
defined stabilisation period; average and peak CPU during the defined workload;
energy over the defined window on battery-powered platforms; installed and
compressed package size.

Reports record device or machine identity, OS build, baseline runtime version,
GPU, thermal state, refresh rate, Migo revision, product profile, workload
revision, cold/warm policy, run count, and timestamps.

### 7.3 Structural Performance Requirements

These are enforced by tests, not by inspection:

- **Bounded hot paths.** No unbounded queue growth under saturation. Terminal
  transitions retain reserved capacity and supersede replaceable work rather
  than being dropped.
- **Zero steady-state allocation.** No per-event heap allocation on any steady
  hot path, including the BLE notification path named in Section 6.1. Each covered
  path requires an allocation-count regression test: a test that counts actual
  allocations during a burst of events and fails when the count is non-zero.

  **The counting mechanism exists.** `engine/testing/alloc-probe` is a dev-only
  crate holding a `GlobalAlloc` that counts allocations, reallocations and frees per
  thread, plus `assert_no_steady_state_allocation`, which runs a warm-up phase, then
  a measured burst, and fails on a non-zero count while naming the path and the
  counts. Three properties make it a gate rather than a decoration:

  - Reallocation counts as an allocation event. A container outgrowing its reserved
    capacity never calls `alloc`, so a counter blind to resizes would miss a
    `with_capacity` that became a `new`.
  - The counters are per thread, because `cargo test` runs tests concurrently
    against one global allocator.
  - Every burst first performs a known allocation and insists on observing it.
    Without that, deleting one `#[global_allocator]` line would turn every gate in
    that binary into a permanent silent pass. The probe crate's own unit-test binary
    installs no counting allocator on purpose, which makes it the negative control
    proving the refusal fires; `tests/harness.rs` installs one and is the positive
    control.

  The allocator is reached only through `[dev-dependencies]`, so a shipped crate
  cannot pull a `#[global_allocator]` into a cdylib — cargo enforces that, rather
  than a comment asking for it.

  **Covered so far:** the ordered host queue (`host_channel`, across coalescible
  motion, reliable and terminal transitions, and the drain), the input payload pool,
  the per-`fillText` text texture cache hit, the decoded-image cache's lookup and
  its pin/unpin pair, the per-call image texture resolve above it that
  `texSubImage2D(image)` takes, the render command path's two enqueues — the
  per-command one and the batched submit — the audio graph's per-quantum
  render on the output thread, and **the frame boundary itself**: building a frame
  packet and running both phases of its execution, which reaches the heap zero
  times in steady state now that the packet's op vector is pooled, the phase
  reorder no longer materialises a reordered packet, and the reorder's admission
  check no longer builds two hash sets per frame. What
  `scripts/test-input-transport-contract.sh` does *not* do is still worth stating,
  because it is the reason this requirement was mis-recorded as satisfied for so
  long: it greps the sources for structural properties — `VecDeque::with_capacity(`,
  a fixed payload-pool capacity formula, a non-zero reliable reserve, the absence of
  `unbounded_channel` — which assert the code is *written* not to allocate. That is
  inspection wearing a test's clothing; it cannot observe an allocation and cannot
  fail when one appears. It now additionally requires that the real gates exist,
  since deleting a test is the one failure a test cannot report about itself.

  **Not covered, and named rather than implied.** The BLE notification path's Rust
  half is `cfg(target_os = "android")`, so a host test binary never compiles it and
  the gate cannot run there; its Java half needs a JVM mechanism entirely, because a
  Rust allocator observes nothing the JVM allocates. The audio path is measured at
  its graph render only; `audio_thread`'s scheduling, `output`'s device handoff and
  `streaming`'s refill are not. Within the frame boundary, the render path's three
  per-frame canvas-id sets are one gated type, but its two remaining call sites —
  the WebGL batch executor and the packet builder — carry no gate of their own,
  because one needs a live GL context and the other a pool no neighbouring test is
  taking from; and that type spills to the heap on a scene with more than 32
  distinct Canvas2D targets in one packet. No path may be recorded as satisfying
  this requirement without a burst test named against it.

  **What the gates cannot see, stated because it bounds the claim.** A burst counts
  allocations, so a *lost* pooled vector — a deallocation — is invisible to it by
  construction. That failure mode is removed structurally rather than gated: a
  command vector is a loan that returns itself when dropped, so there is no recycle
  call to forget. Mutation confirms both halves of that sentence — reverting a loan
  to a plain vector fails the frame gate, while a deliberate `mem::forget` of one
  fails nothing. The type's guarantee is about omission, not about a caller that
  decides to keep what it borrowed.

  **What applying it to the decoded-image cache found**, since it is the second
  instance of one shape and that is what makes it worth stating: a pin recorded in a
  map keyed *beside* the cache needs an owned key to record it and drops that key
  again when the count falls to zero, so an alias taken and released cost one
  allocation and one free per event. The text texture cache paid it per `fillText`;
  this cache paid it per alias, and the fix is the same one — the count belongs on
  the entry. It is not the same *change*, though, and reading it as one would have
  regressed the cache: this cache must accept a pin for a key that is not resident
  yet, because an alias is established before its decode finishes. So the count on
  the entry is paired with a reservation table for exactly the keys an entry cannot
  hold, with one adoption point and one hand-back point, and a key is never in both.

  **And what the layer above it found, which is the same lesson at a boundary
  rather than inside one.** The alias table and the decoded-bytes cache spoke two
  key shapes for the same thing — `(path\0WxH, generation)` above,
  `(path, generation, w, h)` below — so every crossing rebuilt the key. Two owned
  keys per call, on a path `texSubImage2D(image)` takes unconditionally: one
  cloning the alias key out of the table, one parsing the mangled suffix back
  apart for the cache below. Making the two sides share the *type* deletes the
  conversion instead of making it cheaper, and leaves the alias key borrowable —
  so the lookup runs under the alias lock and copies nothing. The lock nesting
  that makes the borrow live long enough is the order this code already took
  everywhere else, and it cannot be inverted: `migo-io` does not depend on
  `runtime-v8`, so nothing holding the decoded-bytes lock can reach an alias
  table.
- **No cross-session lock on a per-event path.** Each covered path requires a
  contention regression test that fails when a per-event operation acquires a
  lock shared beyond its own session.

  **The mechanism exists for the Rust paths.** `engine/testing/contention-probe`
  holds the shared lock in *write* mode and requires the per-event operation, run on
  a thread of its own, to complete anyway. Contention is manufactured rather than
  waited for, so an *uncontended* acquisition — which a load test cannot see — fails
  the gate too. Four details are load-bearing:

  - **A write guard, not a read guard.** An `RwLock` admits concurrent readers, so a
    held read guard lets a per-event `read()` straight through.
  - **The operation runs on another thread.** On the guard holder's own thread a
    re-entrant acquisition parks forever, hanging the suite instead of reporting.
  - **A body that panics is re-raised as its own failure**, never reported as a
    block, because fidelity assertions belong in the body.
  - **Only one gate runs at a time.** Two gates holding two different process-wide
    locks each blame *their* lock for the other's guard; the operation really did
    block, on something the report does not name.

  The waiting bound cannot produce a false pass: shortening it can only make a
  correct path look blocked, which fails closed.

  **Covered:** the per-event input send, gated separately against the host registry
  and the debug-stats registry, and the per-frame text cache hit against the session
  registry. Applying it showed the input send acquiring the process-wide stats
  registry on every event — the very path this section recorded as satisfied — and
  the stats handle is now captured at bring-up alongside the queue and the payload
  pools.

  **No such test exists yet for the permission gate**, which is the one this
  requirement was first written for. The first attempt was withdrawn because it was
  provably unable to fail — the code path it exercised took the shared lock inside
  the very helper the test called, so the test passed with and without the property.
  Its replacement is designed around `ThreadMXBean` blocked-time and is tracked as
  task 5.1; that half is JVM-side and the Rust probe says nothing about it. The BLE
  notification path's Rust half is `cfg(target_os = "android")`, so a host test
  binary never compiles it either.
- **No CPU-bound work on an executor sessions share.** Each covered path requires a
  regression test that fails when a step occupies an executor another session's work
  is waiting on.

  **The mechanism exists.** `engine/testing/executor-probe` spawns the step under
  test onto the shared executor, waits for it to announce that its CPU-bound body has
  begun, and only then spawns a co-tenant task which must run while the step is still
  in flight. The step blocks until the co-tenant releases it, so the co-tenant's
  progress is a precondition of the step finishing rather than a duration measured
  against a clock — which matters because how long a decode occupies a worker depends
  on the chunk it was handed, not on anything a threshold could name. Four details are
  load-bearing:

  - **The bound is the step's own.** This defect's signature is a deadlock, not a
    wrong value: on a single-worker runtime a task spawned after an inline CPU step is
    never polled at all. A gate that waited from outside would hang the suite instead
    of reporting, so the step gives up on its own deadline and the failure is named.
  - **The step records the release it received**, rather than the gate reading a flag
    once the future is done. Under the defect the co-tenant runs the instant the step
    stops occupying the worker, so a flag read afterwards is a coin flip — and with
    the recording moved to the co-tenant, both inline-step controls pass.
  - **Every worker but one is occupied first.** A gate written for one worker would
    pass an inline step on a two-worker runtime, which is the same defect with more
    room. The fillers wait unbounded and are released by the gate letting go of their
    senders: sharing the step's deadline would free a worker at the moment the step
    was still waiting, and an occupying step would pass.
  - **A body that panics is re-raised as its own failure**, never reported as
    occupancy, because fidelity assertions belong in the body.

  Shortening the wait can only make a correct step look occupying, which fails closed.

  **Covered:** the audio streaming MP3 decode, against the process-wide streaming
  worker. Applying it is what moved that decode off the shared worker — it had been
  running inline in the download task, so one game's decode stalled every other game's
  download for as long as the decode took. What carries the fix is `OffWorker<T>`,
  which owns the decoder behind a private field with no accessor: reaching it outside a
  blocking step is a compile error rather than a rule.

  **The shared IO executor's per-host fairness needed a different mechanism, and
  saying which took reading it rather than assuming.** That executor is not a
  tokio runtime — it is a fixed set of OS worker threads behind a condvar, with a
  round-robin lane per host and a cap on how many workers one host may hold while
  another has work queued. So the probe above does not apply: it spawns onto a
  runtime, and its property is *occupancy by CPU work*, which is not the property
  here. The property here is that a worker freed under contention goes to the host
  that is not already over its cap.

  It is now gated by a test against the real executor —
  `a_worker_freed_under_contention_goes_to_the_host_that_is_not_hogging_it` — built
  on the same principle as the two probes even though it shares no code with them:
  manufacture the adversarial condition rather than wait for it. One host fills
  every worker and keeps a backlog, the other submits one job, and exactly one
  worker is freed. Three details are load-bearing, and two are the probes' lessons
  restated: the flooding jobs are released by the test alone and share no deadline
  with the neighbour's wait, because a timeout that freed a worker would hand an
  unfair executor the very thing the neighbour was waiting for; saturation is
  *asserted* before the neighbour submits, since a neighbour handed an idle worker
  proves nothing; and exactly one permit is released rather than a broadcast,
  because freeing every worker asks the dispatcher nothing.

  The queue's own tests already drove this policy directly against `QueueState`,
  so the question was whether a second gate pins anything. It does, and one mutant
  separates them: giving every submitted job the same host token — the plumbing
  between a registration and the queue — fails only the new test, because a test
  that pushes tokens by hand cannot see the path from `submit` to dispatch.
  Removing the cap fails both, which is the same policy seen at two levels rather
  than two guards on one case.

  **One mutant is not usable here, and that is a fact about the harness.** Making
  a completion stop releasing its host and class slots deadlocks the executor's own
  shutdown — workers park with pending work that can never dispatch, and `close`
  cannot drain them — so the suite hangs instead of reporting. It is recorded
  rather than counted.
- **Idle quiescence.** No polling loop and no fixed-interval wakeup when idle.
  Frame delivery is demand-driven. Measured as wakeups per second at idle,
  against a per-platform ceiling recorded in the versioned threshold file
  alongside the other baselines.
- **No redundant presentation copy.** The path from the rendered surface to the
  platform surface performs no additional full-frame copy. The path is documented
  per platform — Android Surface, X11 and Wayland EGL, ANGLE, and
  `OHNativeWindow` — and asserted where the platform allows observation.
- **No steady-state growth.** Resident memory does not grow across a defined
  long-running workload.

### 7.4 Gate Semantics

Baseline thresholds are versioned and reviewed. A missing metric, a zero-sample
report, an absent report, or a report from another revision is a failure and
never a skip. Regressions beyond the approved threshold fail the candidate.
README performance tables are generated from committed immutable summaries
linked to raw reports, and marketing copy carries no unqualified comparative
claim without them.

Emulator measurements are recorded as emulator measurements. They never satisfy
a device performance gate.

**A platform-conditional path is unverified until its own target compiles.** Host
`cargo check`, `cargo test` and `cargo clippy` skip `cfg(android)` code — and the
same holds for every other target's conditional code — so a report resting on them
covers the portable tree only and must say so. This is not hypothetical: the branch
carried three Android compile errors for several sessions, on the touch path, session
teardown and the permission gate, while host runs stayed green. Any change touching
conditional code names the target build that compiled it.

`scripts/verify-change.sh` is what produces that sentence. It derives the required
targets from the changed files, inheriting `cfg` conditions down the module tree
because a file selected by a conditional need not contain one, and reports a target
it cannot build as NOT PROVEN rather than skipping it. A target build it does run is
named with the command that ran it, which is the form this requirement asks for.

## 8. Packaging, Provenance, And Reproducibility

All inherited Phase B requirements apply to four platforms. Additionally:

- HarmonyOS gains a V8 component manifest binding the shipped archive to a
  source revision and GN argument set. The current HarmonyOS package manifest
  records the absence of this binding as a known gap; that gap is a release
  blocker.
- Every `--allow-multiple-definition` relaxation is removed, including the two
  HarmonyOS target entries in `engine/.cargo/config.toml`. Duplicate strong
  global symbols fail the link.
- Content-addressed materialisation of V8, Skia, and ANGLE inputs covers the
  HarmonyOS build path, which currently builds V8 from an external checkout via
  `scripts/build-v8-ohos.sh`.
- Archives are deterministic under `SOURCE_DATE_EPOCH` on all four platforms and
  two builds of the same source and verified inputs compare byte-for-byte equal.
- The HarmonyOS Rust targets are pinned in `engine/rust-toolchain.toml` and
  build without unstable standard-library flags. Any change that reintroduces a
  nightly requirement is a release blocker.

## 9. Helper Repository Contracts

An asset that is untracked, undated, or unattributable to a source revision can
never satisfy a delivery gate.

- **`migo-examples`** is the integration surface of record. It gains a
  HarmonyOS example covering both the HAR consumer and the C ABI consumer,
  collapses to the single version pin from Section 4, keeps resolver trust
  fail-closed, and carries the integration parity markers from Section 3.3 for
  every platform and tier.
- **`migo-bench`** becomes the four-platform performance collector. It currently
  drives Android only through `adb`, with one recorded device tier. It must
  support all four baselines from Section 7.1, key every baseline to the unified
  product version rather than a git SHA, and cover high, mid, and low device
  tiers on mobile. A missing tier is a failure, not a pending row.
- **`migo-conformance`** is the cross-platform behavioural equivalence suite. It
  has runners for the Linux player and Android only; Windows and HarmonyOS
  runners are required, expectation files must be present and attributable, and
  rasterisation coverage requires golden references with a declared tolerance.
- **`migo-test-suite`** is the mini-game API compatibility suite. Its recorded
  baselines are currently directory scaffolding with no dated results, and
  render specifications exist only as untracked working-tree files. Before it
  can gate anything, those specifications are committed and baselines record
  date, device, platform, and Migo revision.
- **`migo-android-demo`** replaces its hard-coded
  `../../migo/platforms/android/dist/migo-debug.aar` path with resolution of a
  released artifact at the unified version.

## 10. CI And Gates

All inherited fail-closed rules apply. Additionally:

- HarmonyOS runs on a Linux runner for everything that does not need DevEco: the
  static library build for both architectures and all three OHOS contract
  scripts. Those scripts already verify built artifacts rather than source text
  and already exit non-zero when the SDK is absent.
- HAP assembly currently requires DevEco on Windows and cannot run on a Linux
  runner. HAP and emulator qualification therefore run locally and emit a
  revision-stamped evidence record. CI verifies that record's presence,
  freshness, and source revision, and fails when it is missing, stale, or from
  another revision. This keeps the non-CI path fail-closed instead of silently
  skipped. A self-hosted HarmonyOS runner may later absorb this gate; until then
  the evidence check is mandatory.
- The integration parity gate from Section 3.3 runs against `migo-examples`.
- The release-candidate assembly job depends on every required platform job, the
  parity gate, the conformance gate, and the performance gate.
- Toolchain pinning extends to the OHOS SDK and DevEco versions.

## 11. Public Documentation

All inherited documentation rules apply. Additionally:

- The HarmonyOS support statement distinguishes the two coverage paths in
  Section 2.4 explicitly. A single undifferentiated "supports HarmonyOS" claim
  is prohibited.
- The compatibility matrix carries a per-platform verified/planned status for
  each capability. HarmonyOS rendering, audio playback, and touch and pointer
  input are listed as delivered once verified; HarmonyOS sensors, camera, video,
  audio recording, and BLE are listed as planned until implemented and tested.
- OpenHarmony is listed as planned. The stale non-goal line in the inherited
  specification is removed by this document, and `README.md` is corrected so its
  existing OpenHarmony claim matches the delivered scope.
- The integration guide documents all three tiers per platform and states which
  tier each example demonstrates. It also documents the canonical host callback
  set once, platform-neutrally, with a per-platform mapping table and the
  documented behaviour of leaving each capability unregistered.
- The product remains Business Source License 1.1 and is described as
  source-available, never as OSI open source.

## 12. External Dependency Register

These cannot be resolved from the current workstation. Each is a hard gate; none
converts to a skip.

1. **HarmonyOS NEXT `arm64-v8a` device.** Only `x86_64` emulator verification
   exists. Until a device is available, HarmonyOS `arm64-v8a` may be built,
   symbol-audited, and emulator-verified, but must not be announced as
   delivery-ready, and its performance row stays empty rather than being filled
   from emulator data.
2. **HarmonyOS 4.x device**, to substantiate the claim that the Android product
   serves that tier.
3. **Android mid-tier and low-tier devices.** The only recorded benchmark device
   is one high-end handset.
4. **OHOS SDK and DevEco installation** on the build host, and a decision about a
   self-hosted HarmonyOS CI runner.
5. **Linux and Windows physical machines** are available and are not blockers.

The absent Android `aarch64` V8 archive
(`engine/third_party/rusty_v8/aarch64/librusty_v8.a`) is deliberately **not**
listed here. It is not an external dependency: `scripts/build-v8-android.sh
aarch64` builds it from the local `rusty_v8` source tree, which is present beside
this repository. It is the first task of Phase 1 because it blocks every Android
build today. Two details must be recorded when it runs: the script's default
`RUSTY_V8_SRC` does not match this workstation, so the correct source path is
passed explicitly, and the source tree's submodules must be initialised or the
script fails closed on a missing `v8/include/v8-version.h`. The resulting archive
is bound to a source revision and GN argument set through the existing V8
component manifest flow; a host archive or an empty archive is never a
substitute.

## 13. Phase Order

This document is a delivery specification, not a single implementation plan. It
decomposes into a delivery ledger with one detailed test-driven plan per task,
following the pattern already established by the inherited specification. No
phase may compensate for an unverified invariant in an earlier phase.

Correctness first, then hermetic builds, then native qualification, then the
integration facades, then consumers, then performance and public material.

1. **Phase 0 — Correctness.** Close inherited A1 through A13, the permission-path
   debt in Section 6.1, and the HarmonyOS correctness items in Section 6.2.
2. **Phase 1 — Hermetic builds and packages.** Build the Android `aarch64` V8
   archive first, since it blocks every Android build. Then Section 4, Section 5,
   and Section 8 across four platforms, including HarmonyOS audio through OHAudio,
   the HarmonyOS V8 provenance binding, and removal of every duplicate-symbol
   relaxation.
3. **Phase 2 — Native qualification.** Every platform reaches first frame,
   survives resize, surface recreation, and input, and exits with no live Migo
   thread, from a clean external consumer.
4. **Phase 3 — Integration contract.** Tier 1 HarmonyOS HAR, Tier 2
   cross-platform Qt 6 kit and header-only wrapper, the canonical host callback
   set from Section 3.4 on all four platforms with its reachability and
   unregistered-settlement tests, facade contract tests, and the integration
   parity gate.
5. **Phase 4 — Consumers.** Helper repository contracts in Section 9, resolver
   and relay hardening, and native CI lanes for every example.
6. **Phase 5 — Performance and release material.** Section 7 evidence on real
   devices and machines, documentation rewrite, candidate assembly, independent
   final audit, and a local candidate commit.

## 14. Definition Of Done

Migo is deliverable only when all of the following hold:

- Android, Linux, Windows, and HarmonyOS build from clean documented
  environments.
- Every artifact in Section 5 exists and verifies.
- Repeated equivalent builds are byte-for-byte reproducible on all four
  platforms.
- No release build uses a duplicate-symbol relaxation, a Rust-skipping shortcut,
  stale native bytes, wall-clock metadata, or warm-cache discovery.
- Engine destruction leaves no thread executing Migo code on any platform.
- Surface release permits immediate release of the documented host resource on
  all four platforms, including `OHNativeWindow`.
- Reattachment either works within the stated compatibility class or fails
  synchronously, with a HarmonyOS platform identity defined.
- Terminal input state converges under saturation, and multi-touch is verified on
  HarmonyOS.
- HarmonyOS delivers rendering, audio playback, and touch and pointer input, and
  its declared permissions do not exceed its honoured capabilities.
- Every protected platform operation has a declaration appropriate to its product
  profile and explicit runtime permission and revocation handling, on Android and
  HarmonyOS alike.
- The integration parity gate passes for every platform and tier, with committed
  baselines.
- Every platform exposes the whole canonical host callback set, every registered
  capability has a reachability test from real content, and every capability has a
  tested unregistered settlement path.
- Two Sessions running two different games concurrently do not interfere, proven
  by behavioural two-session tests rather than by reading, with every defect in
  Section 6.4 fixed and regression-tested.
- Every structural performance requirement in Section 7.3 has a passing
  regression test.
- Device performance evidence exists for every mobile platform and machine
  evidence for every desktop platform, with no metric filled from an emulator.
- Conformance runners exist and pass for all four platforms with attributable
  expectations.
- Packages carry licences, notices, SBOM, manifests, checksums, and trusted
  provenance hooks.
- README, build documentation, platform documentation, changelog, and examples
  describe the same version, support matrix, and integration flow, and the
  HarmonyOS statement distinguishes the two coverage paths.
- All helper repositories are clean, tracked, and pinned to the unified version.
- Every implementation commit remains local until the user chooses to push.

## 15. Explicit Non-Goals

- Publishing a public release, pushing commits, or creating tags.
- iOS, macOS, WinUI, or non-x86_64 desktop delivery.
- OpenHarmony distributions as a delivered target in this candidate.
- HarmonyOS sensors, camera, video, audio recording, and BLE. HarmonyOS audio
  playback is in scope, not a non-goal.
- Full browser DOM, CSS, HTML layout, browser extensions, or Chromium API
  compatibility.
- Productionising the development authentication relay.
- Changing Business Source License 1.1 to an OSI licence.
