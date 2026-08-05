#!/usr/bin/env bash
# =============================================================================
# Generate a V8 startup snapshot for one Android ABI (the "B method").
# =============================================================================
#
# V8 startup snapshots are PLATFORM-bound (OS + CPU arch): a snapshot serializes
# a live V8 heap, so it MUST be produced by the same android-<arch> V8 the .so
# links. We therefore cross-compile `migo-snapshot-gen` to the target ABI
# (linking the committed android V8 archive) and RUN it on that ABI's
# emulator/device (Deno issue #27496 approach), then pull the result into
#   engine/crates/runtime-v8/snapshots/SNAPSHOT-<profile>-<arch>.bin
# which runtime-v8/build.rs embeds (android targets only) at .so build time.
#
# Usage:
#   scripts/gen-snapshot.sh <x86_64|arm64> [--product-profile full|slim]
#                           [--snapshot-kind host|worker]
#                           [--device SERIAL] [--keep]
#
#   x86_64  -> run on an x86_64 Android emulator (or CI; see build-snapshot.yml)
#   arm64   -> run on a real arm64 device (hosted CI has no arm64 KVM)
#
#   --device SERIAL   adb serial to use (else: $ANDROID_SERIAL, else the only
#                     connected device, else error).
#   --keep            leave the pushed binary/lib on the device afterwards.
#
# Environment:
#   ANDROID_NDK_HOME  NDK root (default: ~/Android/Ndk)
#   ADB               adb path (default: ~/Android/Sdk/platform-tools/adb, then PATH)
#
# Output:
#   engine/crates/runtime-v8/snapshots/SNAPSHOT-<profile>-<arch>.bin
#   engine/crates/runtime-v8/snapshots/SNAPSHOT-<profile>-<arch>.bin.manifest.json
#
# After generating BOTH ABIs, build with embedding:
#   bash scripts/build-aar.sh release arm64-v8a x86_64
# =============================================================================
set -euo pipefail

c_info()  { echo -e "\033[0;36m[INFO] $*\033[0m"; }
c_ok()    { echo -e "\033[0;32m[OK]   $*\033[0m"; }
c_err()   { echo -e "\033[0;31m[ERR]  $*\033[0m" >&2; }
die()     { c_err "$*"; exit 1; }

# ---- args -------------------------------------------------------------------
if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then sed -n '2,40p' "$0"; exit 0; fi
ABI="${1:-}"; shift || true
SERIAL="${ANDROID_SERIAL:-}"
KEEP=0
PRODUCT_PROFILE="full"
SNAPSHOT_KIND="host"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --device) SERIAL="${2:-}"; shift 2 ;;
    --product-profile) PRODUCT_PROFILE="${2:-}"; shift 2 ;;
    --product-profile=*) PRODUCT_PROFILE="${1#*=}"; shift ;;
    --snapshot-kind) SNAPSHOT_KIND="${2:-}"; shift 2 ;;
    --snapshot-kind=*) SNAPSHOT_KIND="${1#*=}"; shift ;;
    --keep)   KEEP=1; shift ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    *) die "unknown arg: $1 (try --help)" ;;
  esac
done

case "$PRODUCT_PROFILE" in
  full|slim) ;;
  *) die "invalid --product-profile '$PRODUCT_PROFILE' (expected full|slim)" ;;
esac
case "$SNAPSHOT_KIND" in
  host|worker) ;;
  *) die "invalid snapshot kind '$SNAPSHOT_KIND' (expected host|worker)" ;;
esac
if [[ "$SNAPSHOT_KIND" == "worker" && "$PRODUCT_PROFILE" != "full" ]]; then
  die "Worker snapshot requires product profile full"
fi

case "$ABI" in
  x86_64)
    NDK_TARGET="x86_64"; TRIPLE="x86_64-linux-android"; RUST_ARCH="x86_64"
    V8_DIR="x86_64"; LIBCXX_TRIPLE="x86_64-linux-android"; NEEDS_BUILTINS=0 ;;
  arm64|arm64-v8a|aarch64)
    ABI="arm64"
    NDK_TARGET="arm64-v8a"; TRIPLE="aarch64-linux-android"; RUST_ARCH="aarch64"
    V8_DIR="aarch64"; LIBCXX_TRIPLE="aarch64-linux-android"; NEEDS_BUILTINS=1 ;;
  *) die "usage: gen-snapshot.sh <x86_64|arm64> [--product-profile full|slim] [--device SERIAL] [--keep]" ;;
esac

# ---- paths / tools ----------------------------------------------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/engine"
# shellcheck source=scripts/lib/android-ndk.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/android-ndk.sh"
android_ndk_read_pin "$ROOT/contracts/artifact-manifest/android-v8.lock.json" || exit 1
android_ndk_resolve || die "cannot resolve the pinned Android NDK"
NDK="$ANDROID_NDK_HOME"

ADB="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
[[ -x "$ADB" ]] || ADB="$(command -v adb || true)"
[[ -n "$ADB" && -x "$ADB" ]] || die "adb not found (set ADB=/path/to/adb)"

V8_ARCHIVE="$ENGINE/third_party/rusty_v8/$V8_DIR/librusty_v8.a"
V8_BINDING="$ENGINE/third_party/rusty_v8/$V8_DIR/src_binding.rs"
# The archives are build products fetched from a release asset, not tracked, so
# a fresh checkout legitimately has none. Name the command that gets them:
# pointing at LFS, which this repository no longer uses, sends the reader after
# a mechanism that is gone.
[[ -f "$V8_ARCHIVE" ]] || die "android V8 archive missing: $V8_ARCHIVE (run: bash scripts/fetch-v8-archives.sh)"
# Guard against an unresolved Git LFS pointer (a ~130-byte text stub).
# A truncated or partial download is the failure this catches now that the
# archive arrives over HTTP rather than as an LFS pointer; fetch-v8-archives.sh
# verifies the sha256, so reaching here with a small file means it was bypassed.
[[ "$(stat -c %s "$V8_ARCHIVE")" -gt 1000000 ]] || die "V8 archive is too small to be real: $V8_ARCHIVE (run: bash scripts/fetch-v8-archives.sh)"

LIBCXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/$LIBCXX_TRIPLE/libc++_shared.so"
[[ -f "$LIBCXX" ]] || die "libc++_shared.so not found: $LIBCXX"

# ---- device -----------------------------------------------------------------
if [[ -z "$SERIAL" ]]; then
  mapfile -t DEVS < <("$ADB" devices | awk 'NR>1 && $2=="device"{print $1}')
  [[ "${#DEVS[@]}" -eq 1 ]] || die "specify --device SERIAL (connected: ${DEVS[*]:-none})"
  SERIAL="${DEVS[0]}"
fi
ADBD=("$ADB" -s "$SERIAL")
c_info "kind=$SNAPSHOT_KIND  profile=$PRODUCT_PROFILE  ABI=$ABI  device=$SERIAL  NDK=$NDK"

# ---- 1. cross-compile snapshot-gen -----------------------------------------
# RUSTFLAGS here intentionally overrides .cargo/config.toml (snapshot-gen is a
# standalone binary, not the .so). arm64 additionally needs libclang_rt.builtins
# for __clear_cache (V8 cpu-arm64.cc JIT i-cache flush); x86_64 does not.
RUSTFLAGS_COMMON="-C link-arg=-Wl,--allow-multiple-definition -C link-arg=-landroid -C embed-bitcode=no"
if [[ "$NEEDS_BUILTINS" -eq 1 ]]; then
  BUILTINS="$(find "$NDK" -name 'libclang_rt.builtins-aarch64-android.a' 2>/dev/null | head -1)"
  [[ -n "$BUILTINS" ]] || die "libclang_rt.builtins-aarch64-android.a not found under $NDK"
  RUSTFLAGS_COMMON+=" -L $(dirname "$BUILTINS") -l static=clang_rt.builtins-aarch64-android"
fi

c_info "cross-compiling migo-snapshot-gen for $TRIPLE ..."
( cd "$ENGINE"
  RUSTY_V8_ARCHIVE="$V8_ARCHIVE" \
  RUSTY_V8_SRC_BINDING_PATH="$V8_BINDING" \
  RUSTFLAGS="$RUSTFLAGS_COMMON" \
  cargo ndk -t "$NDK_TARGET" --platform 26 build -p migo-snapshot-gen \
    --no-default-features --features "profile-$PRODUCT_PROFILE" --locked )

BIN="$ENGINE/target/$TRIPLE/debug/migo-snapshot-gen"
[[ -f "$BIN" ]] || die "build produced no binary at $BIN"

# ---- 2. run on device -------------------------------------------------------
c_info "pushing to device ..."
"${ADBD[@]}" push "$LIBCXX" /data/local/tmp/libc++_shared.so >/dev/null
"${ADBD[@]}" push "$BIN" /data/local/tmp/migo-snapshot-gen >/dev/null
"${ADBD[@]}" shell chmod 755 /data/local/tmp/migo-snapshot-gen

c_info "generating snapshot on device ..."
"${ADBD[@]}" shell "cd /data/local/tmp && LD_LIBRARY_PATH=/data/local/tmp MIGO_SNAPSHOT_KIND=$SNAPSHOT_KIND MIGO_SNAPSHOT_OUT=/data/local/tmp/SNAPSHOT.bin ./migo-snapshot-gen"

# ---- 3. pull + manifest -----------------------------------------------------
OUT_DIR="$ENGINE/crates/runtime-v8/snapshots"
mkdir -p "$OUT_DIR"
if [[ "$SNAPSHOT_KIND" == "host" ]]; then
  OUT="$OUT_DIR/SNAPSHOT-$PRODUCT_PROFILE-$RUST_ARCH.bin"
else
  OUT="$OUT_DIR/SNAPSHOT-worker-$PRODUCT_PROFILE-$RUST_ARCH.bin"
fi
"${ADBD[@]}" pull /data/local/tmp/SNAPSHOT.bin "$OUT" >/dev/null
[[ -s "$OUT" ]] || die "pulled snapshot is empty"

# Fingerprint manifest (js_sources_sha256 + deno_core_version). Written by the
# shared helper so this on-device arm64 path and CI's x86_64 path emit identical
# manifests — see scripts/write-snapshot-manifest.sh + check-snapshot-freshness.sh.
MANIFEST="$OUT.manifest.json"
bash "$ROOT/scripts/write-snapshot-manifest.sh" \
  "$PRODUCT_PROFILE" "$RUST_ARCH" "$OUT" "$SNAPSHOT_KIND"

# ---- 4. cleanup -------------------------------------------------------------
if [[ "$KEEP" -eq 0 ]]; then
  "${ADBD[@]}" shell rm -f /data/local/tmp/migo-snapshot-gen /data/local/tmp/SNAPSHOT.bin /data/local/tmp/libc++_shared.so || true
fi

c_ok "snapshot -> $OUT  ($(stat -c %s "$OUT") bytes)"
c_ok "manifest -> $MANIFEST"
echo
if [[ "$SNAPSHOT_KIND" == "worker" ]]; then
  c_info "Build candidate: bash scripts/build-aar.sh --product-profile full --worker-snapshot release $NDK_TARGET <other abis...>"
else
  c_info "Build with embedding: bash scripts/build-aar.sh --product-profile $PRODUCT_PROFILE release $NDK_TARGET <other abis...>"
fi
