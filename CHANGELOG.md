# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Runtime generation fencing, callback correlation, and verification lanes
- Per-session isolate support
- BLE notification path and audio realtime gates
- OpenHarmony API floor declaration gate
- Session delivery and verification lanes (A12)

### Fixed
- A canvas the content never sized now follows a surface that was destroyed and
  recreated at a different size, instead of keeping the size derived from the
  previous one. Rotating while the app is in the background takes that path on
  every Android device, and content came back to a `canvas.width`/`height`
  describing the window it was suspended on, stretched across the new one by the
  presentation blit while `wx.getSystemInfoSync()` reported the real extent.
- A canvas the content *did* size with `canvas.width` is no longer moved when the
  surface resizes. It had been rescaled in proportion to the surface, so a game
  that picked a fixed resolution kept drawing in coordinates its own backing
  store no longer had — into a corner of it.

### Changed
- `MigoSurfaceDescriptor.generation` now documents the rule the C ABI already
  enforced: every attach must carry a generation strictly greater than any the
  session has accepted, and a metrics update carries the live attachment's own.
  A host that stamps a constant is refused with `MIGO_ERROR_STALE_SURFACE` from
  its second attach onwards, which any platform that destroys and recreates its
  window — Android on every trip through the background — reaches on the first
  resume.

---

## v0.9.2 (2026-08-13)

One release, one asset naming scheme, one publisher. Every platform is now
built and staged the same way and named `migo-<version>[-capi]-<platform>[-<arch>].<ext>`,
replacing the three schemes v0.9.1 shipped side by side
(`migo-full-release-arm64-v8a.aar`, `migo-sdk-android-arm64-v8a.tar.gz`,
`migo-linux-x86_64.tar.gz`). This is also the first release where the Windows
and OpenHarmony packages are produced by the same reproducible path as
Android and Linux, rather than hand-tarred.

### Added
- `migo-<version>-android.aar` — the single Java/Kotlin AAR (universal, both
  ABIs); the slim and arm64-only AAR variants are no longer published (a
  consumer's own `abiFilters`/App Bundle already owns that choice — see
  `docs/superpowers/specs/2026-08-12-release-artifact-standard-design.md`)
- `migo-<version>-capi-<platform>-<arch>.tar.gz` for `android`, `linux`,
  `windows`, and `ohos`, each with a package manifest and reproducible
  packaging (`scripts/package-sdk.sh`)
- `migo-<version>-sbom.cdx.json`, `SHA256SUMS.txt`, `version.json`
- V8 startup snapshots now embed on Linux (host + worker, full profile), not
  only Android — `runtime-v8/build.rs` dispatches embedding by `(os, arch)`
  instead of an Android-only check. Snapshot filenames carry an OS segment
  (`SNAPSHOT-<profile>-<os>-<arch>.bin`) so android-x86_64 and linux-x86_64,
  previously colliding, are distinct files
- `scripts/test-capi-snapshot-embedding-contract.sh`: proves a shipped C-ABI
  static/shared library actually contains the snapshot bytes its package
  manifest claims, the same property the AAR contract already proved for
  `libmigo.so`
- Windows: `x86_64-pc-windows-msvc` V8 built and its component manifest
  sealed for the first time; the archive (`rusty_v8.lib` + `rusty_v8.dll` +
  import library) is published on the `v8-archives-e6a88b3` release and
  fetchable via `scripts/fetch-v8-archives.sh x86_64-pc-windows-msvc`
- Windows: ANGLE's runtime (`libEGL.dll`, `libGLESv2.dll`,
  `d3dcompiler_47.dll`) is pinned to a verified download
  (`contracts/artifact-manifest/windows-angle.lock.json` +
  `scripts/fetch-windows-angle.sh`) instead of an ad hoc local directory —
  ANGLE publishes no official prebuilt Windows binaries, so these are
  self-hosted on the same release tag the V8 archives use
- OpenHarmony: `librusty_v8-{aarch64,x86_64}-linux-ohos.a` published on
  `v8-archives-e6a88b3`, and OHOS builds in CI (`release-ohos`)
- `scripts/verify-release-assets.sh`: enforces that every published asset is
  covered either by `SHA256SUMS.txt` or its own `.attestation.json`, checked
  against the live GitHub release rather than build intent
- `scripts/test-release-asset-naming-contract.sh` and
  `test-release-asset-ordering-contract.sh` guard the naming scheme and the
  publish job's asset list against drift

### Changed
- `release.yml` restructured per platform: `release-android`, `release-linux`,
  and `release-ohos` build and stage in parallel; a single `publish` job merges
  every platform's staged output, generates one `SHA256SUMS.txt` covering the
  whole release (previously only the Android job's output), and performs one
  upload. Windows is not in CI yet (`build-windows-sdk.sh` needs WSL/`cmd.exe`
  interop a `windows-latest` runner does not have) and is built and uploaded
  by hand until a Windows-native job exists
- The Android V8 archive directories and release asset names moved from bare
  architecture words to full target triples (`aarch64` →
  `aarch64-linux-android`, `x86_64` → `x86_64-linux-android`), matching the
  vocabulary the Linux and OpenHarmony directories already used
- `scripts/publish-release.sh`, whose `required_files` still named v0.9.0-era
  assets and had fully diverged from `release.yml`, is removed

### Fixed
- `SHA256SUMS.txt` no longer silently covers only the Android job's output
  while implying whole-release coverage — see the `release.yml` restructuring
  above
- A release AAR could previously claim an embedded snapshot in its slice
  manifest without the shipped `.so` actually containing it (`build.rs` fails
  safe and only warns on a stale/invalid snapshot); `scripts/test-android-
  snapshot-embedding-contract.sh` and its new C-ABI sibling now read the
  shipped bytes to prove this rather than trusting the manifest

---

## Engine — v0.9.0 (2026-07-28)

First public engine release. Ships the Rust multi-crate engine with a C ABI
(`libmigo_capi`) on four platforms: Android, Linux, Windows, and OpenHarmony.

### Added
- WebAudio-style runtime with `AudioContext`, `AudioBuffer`, and
  `InnerAudioContext` APIs compatible with mini-game style
- Audio decoders for MP3, OGG, and WAV formats; streaming and caching pipeline
- Canvas 2D and WebGL rendering APIs
- File I/O (sync and async), network fetch, and touch input
- C ABI (`migo-capi`) with a stable, documented export surface (`migo_*`)
- Android JNI bindings and AAR packaging (`migo-full-release.aar`,
  `migo-slim-release.aar`); Android demo project in
  [migo-examples](https://github.com/minigame-labs/migo-examples)
- Android C-API SDK tarballs (`migo-sdk-android-arm64-v8a.tar.gz`,
  `migo-sdk-android-x86_64.tar.gz`)
- Linux x86_64 C-API SDK (`migo-sdk-linux-x86_64.tar.gz`); see `linux-sdk-0.1.0`
  below
- Windows x86_64 C-API SDK (`migo-sdk-windows-x86_64.tar.gz`); see
  `windows-sdk-0.1.1` below
- OpenHarmony (aarch64 and x86_64) builds and contract gates; no published
  release yet (see known gaps in `dist/migo-ohos-x86_64/share/migo/ohos-x86_64-manifest.json`)
- Prebuilt V8 archives for Android and Linux distributed via release assets
  (release `v8-archives-e6a88b3`, 2026-07-25)
- `scripts/fetch-v8-archives.sh` fetches and verifies prebuilt V8 archives
  against committed component manifests
- `release/VERSION` as the single version source; `scripts/test-release-version-contract.sh`
  enforces that all build consumers derive from it

### Changed
- Renamed project from `minigame_host` to `migo`
- Renamed SO library from `libminigame_host.so` to `libmigo.so`
- V8 archives moved from Git LFS to release assets to avoid LFS quota exhaustion

---

## Linux SDK — linux-sdk-0.1.0 (2026-07-28)

Packaged alongside `v0.9.0`. The Linux SDK carries its own version series because
it is a separately consumable artifact with its own ABI and loader-floor contract,
distinct from the engine's feature version.

### Added
- `libmigo.so` (versioned, soname `libmigo.so.1`) and `libmigo.a` for
  `x86_64-unknown-linux-gnu`
- glibc 2.31 / GLIBCXX 3.4.28 loader floor, enforced by building against the
  Debian bullseye amd64 sysroot (Chromium's pinned sysroot)
- Export surface controlled by a version script; only the documented `migo_*`
  entry points are exported
- CMake `find_package(migo)` support, `pkg-config` `.pc`, and public headers
- Package manifest (`linux-x86_64-manifest.json`) with sha256 hashes and
  provenance; verified by `scripts/test-linux-sdk-contract.sh`
- Qt 6 host kit (`platforms/linux/host-kit/`) with X11 surface view and managed
  session; gated by `scripts/test-linux-qt-host-kit.sh`

---

## Windows SDK — windows-sdk-0.1.1 (2026-07-29)

The Windows SDK carries its own version series for the same reason as the Linux
SDK. `windows-sdk-0.1.1` supersedes `windows-sdk-0.1.0` (never publicly tagged):
`0.1.0` shipped a DLL that loaded and exported all entry points but could attach
no Win32 surface; `0.1.1` adds the Win32 HWND platform layer.

### Added
- `migo.dll` (x86_64, MSVC) with a `.def`-controlled export surface restricted to
  documented `migo_*` entry points
- `migo.lib` import library, `rusty_v8.dll`, and ANGLE runtime DLLs
  (`libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll`) shipped alongside
- CMake `find_package(migo)` support targeting the MSVC toolchain
- Win32 HWND surface platform layer; the DLL loads and reports
  `MIGO_PLATFORM_WIN32_HWND` as an attachable kind
- Contract gate (`scripts/test-windows-sdk-contract.sh`) that loads the DLL and
  exercises `migo_query_capabilities` to verify surface support is present

---

## Versioning

This project uses [Semantic Versioning](https://semver.org/):

- **MAJOR**: Incompatible API changes
- **MINOR**: New functionality (backward compatible)
- **PATCH**: Bug fixes (backward compatible)

Per-platform SDKs (Linux, Windows) carry their own version series
(`linux-sdk-X.Y.Z`, `windows-sdk-X.Y.Z`) because each is a separately consumable
artifact with its own ABI contract. The engine version (`v0.9.0`) and the
per-platform SDK versions can move independently.

### Pre-1.0 Policy

While the version is below 1.0.0:
- MINOR version bumps may include breaking changes
- PATCH version bumps are backward compatible

[Unreleased]: https://github.com/minigame-labs/migo/compare/v0.9.2...HEAD
[v0.9.2]: https://github.com/minigame-labs/migo/releases/tag/v0.9.2
[v0.9.0]: https://github.com/minigame-labs/migo/releases/tag/v0.9.0
[linux-sdk-0.1.0]: https://github.com/minigame-labs/migo/releases/tag/linux-sdk-0.1.0
[windows-sdk-0.1.1]: https://github.com/minigame-labs/migo/releases/tag/windows-sdk-0.1.1
