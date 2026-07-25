# Migo Build Guide

How to set up a development environment and build Migo on **Linux**,
**macOS**, and **Windows**. Covers both host-side Rust builds
(unit tests, smoke checks) and Android cross-compilation
(`libmigo.so`, AAR packaging).

> This file describes *how* to build the engine from source.

---

## Contents

- [Overview](#overview)
- [Prerequisites (all platforms)](#prerequisites-all-platforms)
- [Platform setup](#platform-setup)
  - [Linux (Ubuntu/Debian/WSL2)](#linux-ubuntudebian-wsl2)
  - [macOS (Apple Silicon + Intel)](#macos-apple-silicon--intel)
  - [Windows 10/11](#windows-1011)
- [Build workflows](#build-workflows)
  - [1. Host Rust tests (fast)](#1-host-rust-tests-fast)
  - [2. Android shared library (`libmigo.so`)](#2-android-shared-library-libmigoso)
  - [3. Android AAR (for app integration)](#3-android-aar-for-app-integration)
- [Troubleshooting](#troubleshooting)
- [Project invariants](#project-invariants)

---

## Overview

Migo is a Rust multi-crate engine (`engine/crates/`) that ships as a
native Android library (`libmigo.so`) plus a Java SDK (`platforms/
android/library/`). Three main outputs:

| Output | Tool | Primary use |
|---|---|---|
| Host binaries + unit tests | `cargo test` / `cargo build` | Develop & test on the dev machine |
| `libmigo.so` (arm64 / x86_64) | `scripts/build-android-so.{sh,ps1}` | Drop-in replacement of the native lib |
| AAR | `scripts/build-aar.{sh,ps1}` | Integrate as a Gradle dependency |

**Minimum Android API**: `26` (Android 8.0 Oreo). Locked by
`skia-bindings 0.93` — see `scripts/build-android-so.sh` for the full
rationale.

---

## Prerequisites (all platforms)

### Rust toolchain

```bash
# Install rustup from https://rustup.rs/ (or your package manager).
# The toolchain file pins to stable; Rust 1.80+ required for edition 2024.
rustup show                       # verify the pin is respected
```

Add Android targets:

```bash
rustup target add aarch64-linux-android
rustup target add x86_64-linux-android
```

Install `cargo-ndk` (cross-compile helper):

```bash
cargo install cargo-ndk
```

### Android NDK (only if building `libmigo.so`/AAR)

- **Version**: r23 through r27 have been verified. r23b/r25 are the
  two most battle-tested; we regularly build on `r23.2.8568313`.
- **Required env var**: `ANDROID_NDK_HOME` pointing to the NDK root
  (directory containing `toolchains/llvm/prebuilt/`).

### Skia source-build tooling

Migo uses Skia via `skia-safe 0.93` with the `binary-cache` feature.
**Android targets have no prebuilt binary** in the upstream
`rust-skia/skia-binaries` GitHub releases (only Linux/macOS/Windows
x86_64/arm64 *host* targets do). This means an Android build will:

1. Attempt the binary download → receive `HTTP 404` (expected).
2. Fall back to a **from-source Skia compile** (`STARTING A FULL BUILD`).

The from-source path needs:

| Tool | Minimum version | Why |
|---|---|---|
| `python3` | 3.8+ | Skia's `gn` generator scripts |
| `ninja`   | 1.10+ | Build runner (actually builds .o files) |
| `git`     | any modern | Skia's `gclient sync` pulls submodules |
| disk      | ~10 GB free | Skia source + build intermediates |
| RAM       | ~6 GB peak  | Some C++ translation units are big |

### JDK (only for AAR builds)

JDK 17+ is recommended. Gradle 8.4 (bundled in `platforms/android/`)
has been verified on 17 and 21.

---

## Platform setup

### Linux (Ubuntu/Debian/WSL2)

```bash
# System packages
sudo apt update
sudo apt install -y python3 git curl build-essential pkg-config \
    libfontconfig1-dev libfreetype6-dev libegl1-mesa-dev default-jdk

# ninja — dev-setup-skia.sh installs a pinned prebuilt to ~/.local/bin
source scripts/dev-setup-skia.sh   # NOTE: `source`, not `bash`

# Android NDK — via sdkmanager or Android Studio SDK Manager
# Then:
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/23.2.8568313"
# (persist it in your shell rc file)

# Install cargo-ndk + Rust targets
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

**WSL2 specific — RAM knob**: Skia's from-source build spikes to
~6 GB. WSL2 defaults to 50% host RAM; if your machine has 8 GB you'll
OOM. Create `%USERPROFILE%\.wslconfig` on the Windows side:

```ini
[wsl2]
memory=12GB
swap=8GB
```

Then `wsl --shutdown` and reopen.

### macOS (Apple Silicon + Intel)

```bash
# Via Homebrew
brew install python@3.12 ninja git openjdk@17
export JAVA_HOME=$(/usr/libexec/java_home -v 17)

# Android NDK — via sdkmanager (installed by Android Studio)
# or direct download from https://developer.android.com/ndk/downloads
export ANDROID_NDK_HOME="$HOME/Library/Android/sdk/ndk/23.2.8568313"

# Rust targets + cargo-ndk
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

Apple Silicon note: the NDK you download **must** contain
`toolchains/llvm/prebuilt/darwin-arm64` (recent NDKs do; older r21
bundles only `darwin-x86_64` and require Rosetta to run).

### Windows 10/11

```powershell
# Rust toolchain — download rustup-init.exe from https://rustup.rs/
# After install, restart the terminal and verify:
rustc --version
cargo --version

# Rust targets
rustup target add aarch64-linux-android
rustup target add x86_64-linux-android

# cargo-ndk
cargo install cargo-ndk

# Skia from-source dependencies
winget install Python.Python.3.12         # or installer from python.org
winget install Ninja-build.Ninja
winget install Git.Git

# Android NDK — install via Android Studio SDK Manager
#   SDK Tools → "NDK (Side by side)" v23.x or v25.x
# Default path ends up at %LOCALAPPDATA%\Android\Sdk\ndk\<version>

# Set ANDROID_NDK_HOME at user scope (persists across sessions)
[Environment]::SetEnvironmentVariable(
    "ANDROID_NDK_HOME",
    "$env:LOCALAPPDATA\Android\Sdk\ndk\23.2.8568313",
    "User"
)

# Restart PowerShell, then verify
$env:ANDROID_NDK_HOME
Test-Path "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64"

# JDK 17 (only needed for AAR build)
winget install EclipseAdoptium.Temurin.17.JDK
```

**PowerShell execution policy** — if scripts refuse to run:

```powershell
# One-off: run a single script bypassing policy
powershell -ExecutionPolicy Bypass -File .\scripts\build-android-so.ps1 arm64-v8a release

# Or relax at user scope (only your account)
Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned
```

**Git long-path support** — Skia checkouts include paths > 260 chars:

```powershell
git config --global core.longpaths true
```

---

## Build workflows

### 1. Host Rust tests (fast)

No NDK needed. Runs on the dev machine in seconds.

```bash
cd engine
cargo test --workspace --lib --no-fail-fast
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: **499+ unit tests passing** (some environment-specific
tests may be skipped with `--skip` — see `scripts/ci/run_smoke.sh`).

### 2. Android shared library (`libmigo.so`)

Produces `engine/jniLibs/<abi>/libmigo.so` and `libc++_shared.so`.

**Linux/macOS**:

```bash
# arm64 only, release
bash scripts/build-android-so.sh arm64-v8a release

# Both ABIs
bash scripts/build-android-so.sh all release

# Debug (faster, unoptimised)
bash scripts/build-android-so.sh arm64-v8a debug
```

**Windows**:

```powershell
.\scripts\build-android-so.ps1 arm64-v8a release
.\scripts\build-android-so.ps1 all release
.\scripts\build-android-so.ps1 arm64-v8a           # debug
```

**Expected timings**:

| Scenario | Time |
|---|---|
| Cold build (no cached Skia) | **10–25 min** (Skia source compile dominates) |
| Incremental (Rust-only change) | 1–2 min |
| Both ABIs cold | 20–40 min |

The script prints `[SUCCESS] All Android builds succeeded` at the
end. Resulting files:

```
engine/jniLibs/arm64-v8a/libmigo.so
engine/jniLibs/arm64-v8a/libc++_shared.so
engine/jniLibs/x86_64/libmigo.so
engine/jniLibs/x86_64/libc++_shared.so
```

### 3. Android AAR (for app integration)

Requires step 2 (which it invokes internally) plus JDK 17+.

**Linux/macOS**:

```bash
bash scripts/build-aar.sh release
bash scripts/build-aar.sh release arm64-v8a           # single ABI
```

**Windows**:

```powershell
.\scripts\build-aar.ps1 release
.\scripts\build-aar.ps1 release arm64-v8a
```

Artifact:

```
platforms/android/dist/library-release.aar
```

Skip the Rust step if the `.so` files are already up-to-date:

```bash
bash scripts/build-aar.sh release --skip-rust
```

---

## Troubleshooting

### `curl: (22) The requested URL returned error: 404` on skia-bindings

**Not a failure**. The skia-bindings prebuilt download only supports
host targets; for Android it returns 404 and the build falls back to
`STARTING A FULL BUILD`. If the real failure happens *after* that
line, something in the source-compile chain is missing — check the
next 100 lines of the log:

```bash
# Linux/macOS
bash scripts/build-android-so.sh arm64-v8a release 2>&1 | tee /tmp/build.log
tail -120 /tmp/build.log

# Windows
.\scripts\build-android-so.ps1 arm64-v8a release 2>&1 | Tee-Object build.log
Get-Content build.log -Tail 120
```

Most common root causes, in order:

1. **`ninja: command not found`** — install per
   [platform setup](#platform-setup) or run `source scripts/
   dev-setup-skia.sh` on Linux.
2. **`python3: command not found`** — install Python 3.8+.
3. **OOM during `SkCanvas.cpp` compile** — free up memory or raise
   WSL2 limit.
4. **`gclient sync` timing out** — set a git / network proxy:
   ```bash
   export HTTPS_PROXY=http://127.0.0.1:7890
   export HTTP_PROXY=http://127.0.0.1:7890
   git config --global http.proxy $HTTP_PROXY
   ```

### `error: ANDROID_NDK_HOME is not set`

The env var must be set *and* the NDK directory actually exist:

```bash
# Linux/macOS
echo "$ANDROID_NDK_HOME"
ls "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt"
```

```powershell
# Windows
$env:ANDROID_NDK_HOME
Test-Path "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64"
```

On Windows the env var must be in **User** (not Process) scope if you
want it to persist across terminal restarts. See the setup section
above.

### `libclang_rt.builtins-aarch64-android.a not found`

Some NDK versions lay out the clang builtins differently. Verify it
exists somewhere under your NDK:

```bash
find "$ANDROID_NDK_HOME" -name 'libclang_rt.builtins-aarch64-android.a'
```

If empty, try a different NDK version (r23b or r25c are known-good).

### `libc++_shared.so not found` after build

The `copy libc++_shared.so` step points at
`toolchains/llvm/prebuilt/<host>/sysroot/usr/lib/<target>/libc++_shared.so`.
If your NDK has a different host directory name, either install a
matching NDK or edit the script's host path.

### Gradle: `Could not find gradle-8.4-all.zip`

The AAR script downloads the Gradle wrapper on first run; requires
network access. If behind a firewall:

```bash
export GRADLE_OPTS='-Dhttps.proxyHost=127.0.0.1 -Dhttps.proxyPort=7890'
```

### Tests fail on `sync_storage_mutate_and_info_use_scheduler_worker_path`

This test is known to fail on `master` (literal `key1` is not hex-encoded);
the failure predates the current test author. Skip with
`--skip sync_storage_mutate_and_info_use_scheduler_worker_path`. CI's
`run_smoke.sh` has the skip baked in.

### Tests fail on `mixed_preload_batches_keep_decode_work_running_while_cached_tasks_wait`

Known-flaky timing test, also pre-existing. Skip with the same
mechanism.

---

## Project invariants

Things the build guarantees (verified by `scripts/ci/run_smoke.sh`):

- **Android feature gate**: `zune-image` and `image` crates **never**
  appear in the Android feature graph (the platform decoder is
  BitmapFactory via JNI). Verified by
  `scripts/ci/check_android_feature_gate.sh`.
- **minSdk = 26**: enforced by `ANDROID_API=26` in both
  `build-android-so.{sh,ps1}` and `platforms/android/library/
  build.gradle`.
- **Zero AndroidX dependencies** in `library/build.gradle` (see the
  declaration comment there).
- **Rust `libmigo.so` size** budget: currently ~48 MB arm64 release
  (+2.08% vs the M0 pre-refactor baseline). Significant regressions
  should be justified in the commit.

If any of these change, update **this file** and the corresponding CI
check in lock-step.
