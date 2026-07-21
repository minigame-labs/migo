#!/usr/bin/env bash
# Build the Linux SDK artifacts against the pinned sysroot and stage a package
# a non-cargo host can consume.
#
# Usage: scripts/build-linux-sdk.sh [--prefix DIR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"
TARGET="x86_64-unknown-linux-gnu"

PREFIX="$REPO_ROOT/dist/migo-linux-x86_64"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        *) echo "[linux-sdk] unknown argument: $1" >&2; exit 2 ;;
    esac
done

info() { echo -e "\033[0;36m[linux-sdk] $*\033[0m"; }

# shellcheck source=scripts/lib/linux-sysroot.sh
source "$SCRIPT_DIR/lib/linux-sysroot.sh"
migo_sysroot_export

LINK_DIR="$ENGINE_DIR/target/sysroot-links"
migo_sysroot_link_dir "$LINK_DIR"
export LIBRARY_PATH="$LINK_DIR:${LIBRARY_PATH:-}"
# -B, not just -L: the driver searches -B prefixes for libraries ahead of its
# own built-in directories, which is what keeps `-lstdc++` from resolving to the
# host GCC's copy. A plain -L lands after those built-ins and loses the race.
RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-B$LINK_DIR -C link-arg=-L$LINK_DIR"

# Prefer the archive scripts/build-v8-linux.sh installed: it is the one built
# against the sysroot and configured so it can enter a shared object. The
# rusty_v8_src build directory is only a fallback for a checkout that has not
# run that script yet, and it cannot produce libmigo.so.
V8_SDK_DIR="$ENGINE_DIR/third_party/rusty_v8/x86_64-linux-gnu"
if [[ -f "$V8_SDK_DIR/librusty_v8.a" ]]; then
    V8_ARCHIVE="$V8_SDK_DIR/librusty_v8.a"
    V8_BINDING="$V8_SDK_DIR/src_binding.rs"
    V8_MANIFEST="$V8_SDK_DIR/component-manifest.json"
else
    V8_DIR="${MIGO_HOST_V8_DIR:-$REPO_ROOT/../rusty_v8_src/target/$TARGET/release/gn_out}"
    V8_ARCHIVE="$V8_DIR/obj/librusty_v8.a"
    V8_BINDING="$V8_DIR/src_binding.rs"
    V8_MANIFEST=""
    echo "[linux-sdk] warning: using $V8_ARCHIVE; run scripts/build-v8-linux.sh" >&2
    echo "[linux-sdk] warning: for an archive that meets the floor and can build libmigo.so" >&2
fi
[[ -f "$V8_ARCHIVE" ]] || { echo "[linux-sdk] no V8 archive at $V8_ARCHIVE" >&2; exit 1; }
[[ "$(stat -c %s "$V8_ARCHIVE")" -gt 1000000 ]] \
    || { echo "[linux-sdk] V8 archive looks like a stub or LFS pointer" >&2; exit 1; }
export RUSTY_V8_ARCHIVE="$V8_ARCHIVE"
export RUSTY_V8_SRC_BINDING_PATH="$V8_BINDING"

# The NDK must not be visible, or skia-bindings picks its toolchain over ours.
unset ANDROID_NDK ANDROID_NDK_HOME

export RUSTFLAGS="${RUSTFLAGS:-} $RUSTFLAGS_SYSROOT_LINK"

# The v8 crate bundles librusty_v8.a into its rlib at build time, and cargo only
# reruns its build script when RUSTY_V8_ARCHIVE's *value* changes -- not when the
# file at that path is replaced. Rebuilding V8 in place therefore leaves cargo
# reusing an rlib containing the previous archive, silently. That cost a full
# debugging cycle: a freshly staged libmigo.so still carried the allocator shim
# that the V8 rebuild had just removed.
V8_STAMP="$ENGINE_DIR/target/$TARGET/.v8-archive-sha256"
V8_SHA="$(sha256sum "$V8_ARCHIVE" | cut -d' ' -f1)"
mkdir -p "$(dirname "$V8_STAMP")"
if [[ ! -f "$V8_STAMP" || "$(cat "$V8_STAMP")" != "$V8_SHA" ]]; then
    info "V8 archive changed; forcing a rebuild of the v8 crate"
    cargo clean --manifest-path "$ENGINE_DIR/Cargo.toml" \
        -p v8 --release --target "$TARGET" 2>/dev/null || true
    printf '%s' "$V8_SHA" > "$V8_STAMP"
fi

info "sysroot: $MIGO_SYSROOT"
info "building capi staticlib (release, $TARGET)"
cargo build --manifest-path "$ENGINE_DIR/Cargo.toml" \
    -p migo-capi --release --target "$TARGET"

info "built: $ENGINE_DIR/target/$TARGET/release/libmigo_capi.a"

# The floor is a property of linked output -- a static archive carries no symbol
# versions of its own. This binary is built with the sysroot toolchain purely so
# there is something authoritative to audit; the out-of-tree consumers built with
# a stock compiler cannot serve, because their own objects legitimately carry the
# build host's newer symbols.
info "building the reference consumer used for the floor audit"
cargo build --manifest-path "$ENGINE_DIR/Cargo.toml" \
    -p migo-c-host-example --release --target "$TARGET"

info "capturing the link line cargo uses"
LINK_NOTE="$ENGINE_DIR/target/$TARGET/release/native-static-libs.txt"
cargo rustc --manifest-path "$ENGINE_DIR/Cargo.toml" \
    -p migo-capi --lib --release --target "$TARGET" --crate-type staticlib -- \
    --print native-static-libs > "$LINK_NOTE" 2>&1
grep -q 'native-static-libs:' "$LINK_NOTE" \
    || { echo "[linux-sdk] cargo did not report native-static-libs" >&2; exit 1; }

VERSION="$(grep -m1 '^version' "$ENGINE_DIR/crates/capi/Cargo.toml" | cut -d'"' -f2)"

# ------------------------------------------------------------
# libmigo.so
#
# Linked here from the static library rather than produced by a cdylib crate,
# for one reason: rustc gives every cdylib a version script of its own listing
# each reachable `#[no_mangle]` symbol, the linker takes the union with ours,
# and the result exported 338 symbols -- `diplomat_*`, `temporal_rs_*` and
# `rusty_v8_*` from dependency crates alongside the 20 documented entry points.
# `--exclude-libs,ALL` does not reach them either. Linking here makes migo.map
# the only version script, so the exported surface is exactly what the headers
# declare.
#
# Two details that are not obvious:
#   * No --whole-archive. The static library carries ICU twice (V8's copy and
#     Skia's), and pulling every member in produces duplicate definitions.
#     `-u` on each entry point gives the linker roots to resolve from, so it
#     pulls the same object set cargo's own link does.
#   * --no-undefined, so a missing symbol fails here rather than at dlopen on a
#     machine we cannot see.
#   * --gc-sections is load-bearing, not an optimisation. skia-bindings compiles
#     one translation unit containing wrappers for JPEG, PDF and pathops, and
#     Skia here is built without those features, so that object references
#     symbols that do not exist (SkJpegEncoder::Encode, SkPDF::AttributeList::*,
#     SkOpBuilder::resolve, ...). An executable survives because the unreferenced
#     wrappers are collected before their references have to resolve; a shared
#     library treats every global as a potential entry point and keeps them. The
#     version script above demotes everything but `migo_*` to local, which is
#     what lets section GC drop them here too.
#
#     This is the failure the C ABI runtime slice recorded as "adding Skia's
#     archives forces skia-bindings' translation unit to be linked whole". The
#     mechanism is different: the object is always linked, and what matters is
#     whether the linker may discard the parts of it nothing calls.
# ------------------------------------------------------------
# lld, not the default GNU ld.
#
# GNU ld produces a malformed symbol-version table for this link: imported
# symbols (snd_pcm_*, sendto, pthread_detach, ...) come out marked as defined in
# *ABS* carrying their originating library's version index, and the resulting
# library cannot be linked against at all -- `invalid version 11 (max 2)` from
# GNU ld, `corrupt input file: version definition index 3 out of bounds` from
# lld. Nothing in the static archive defines those symbols, so the entries are
# the linker's own. lld given the identical inputs produces a well-formed table
# and a library external consumers link cleanly.
#
# The version script cannot simply be dropped to avoid this: without it,
# --gc-sections has nothing to collect and the link fails on the JPEG/PDF
# wrappers described below.
LLD_BIN="${MIGO_LLD:-$(command -v ld.lld 2>/dev/null || true)}"
if [[ -z "$LLD_BIN" ]]; then
    BUNDLED_LLD="$REPO_ROOT/../rusty_v8_src/target/$TARGET/release/clang/bin/ld.lld"
    [[ -x "$BUNDLED_LLD" ]] && LLD_BIN="$BUNDLED_LLD"
fi
if [[ -z "$LLD_BIN" ]]; then
    echo "[linux-sdk] no lld found. Install it (apt-get install lld) or set MIGO_LLD." >&2
    echo "[linux-sdk] A system lld is preferred; the rusty_v8 toolchain copy is only a" >&2
    echo "[linux-sdk] convenience for machines that do not have one." >&2
    echo "[linux-sdk] GNU ld cannot be used here: it emits a malformed symbol-version" >&2
    echo "[linux-sdk] table for this link and the result is unlinkable." >&2
    exit 1
fi

info "linking libmigo.so (linker: $LLD_BIN)"
SO_MAP="$ENGINE_DIR/crates/capi/migo.map"
ENTRY_POINTS=$(grep -ohE '\bmigo_[a-z0-9_]+[[:space:]]*\(' "$REPO_ROOT"/include/migo/*.h \
    | tr -d '( \t' | sort -u)
UNDEF_ARGS=()
for entry in $ENTRY_POINTS; do
    UNDEF_ARGS+=("-Wl,-u,$entry")
done
NATIVE_LIBS=$(sed -n 's/.*native-static-libs: *//p' "$LINK_NOTE" | head -1)
SO_BUILD="$ENGINE_DIR/target/$TARGET/release/libmigo.so.$VERSION"

# shellcheck disable=SC2086  # NATIVE_LIBS is a flag list and must word-split
"$CXX" --ld-path="$LLD_BIN" -shared -o "$SO_BUILD" \
    -Wl,--version-script="$SO_MAP" \
    -Wl,-soname,libmigo.so.1 \
    -Wl,--no-undefined \
    -Wl,--gc-sections \
    "${UNDEF_ARGS[@]}" \
    "$ENGINE_DIR/target/$TARGET/release/libmigo_capi.a" \
    $NATIVE_LIBS \
    -L "$LINK_DIR" -B "$LINK_DIR" \
    --sysroot="$MIGO_SYSROOT"
info "linked: $SO_BUILD ($(stat -c %s "$SO_BUILD") bytes)"

info "staging package at $PREFIX"
rm -rf "$PREFIX"
mkdir -p "$PREFIX/include" "$PREFIX/lib" "$PREFIX/share/migo"
cp -r "$REPO_ROOT/include/migo" "$PREFIX/include/"
cp "$ENGINE_DIR/target/$TARGET/release/libmigo_capi.a" "$PREFIX/lib/libmigo.a"
cp "$SO_BUILD" "$PREFIX/lib/libmigo.so.$VERSION"
# soname -> real file, and the unversioned name a `-lmigo` link resolves.
ln -sf "libmigo.so.$VERSION" "$PREFIX/lib/libmigo.so.1"
ln -sf "libmigo.so.1" "$PREFIX/lib/libmigo.so"

# --shared: `-lmigo` resolves to libmigo.so unless a consumer asks for
# --static, so the CMake imported target has to describe the same default or the
# two integrations would disagree about what `migo::migo` is.
python3 "$SCRIPT_DIR/gen-linux-package-metadata.py" \
    --cargo-output "$LINK_NOTE" --prefix "$PREFIX" --version "$VERSION" --shared

info "recording the artifact manifest"
NEEDED_FILE="$ENGINE_DIR/target/$TARGET/release/dt-needed.txt"
# Taken from libmigo.so: the manifest describes what the shipped library needs,
# not what one particular consumer happened to link.
python3 "$SCRIPT_DIR/abi-floor-audit.py" needed "$SO_BUILD" > "$NEEDED_FILE"
python3 "$SCRIPT_DIR/gen-linux-package-metadata.py" --manifest \
    --prefix "$PREFIX" --version "$VERSION" \
    --needed-from "$NEEDED_FILE" --sysroot "$MIGO_SYSROOT" \
    ${V8_MANIFEST:+--v8-component-manifest "$V8_MANIFEST"}

info "staged:"
find "$PREFIX" -type f -o -type l | sed "s|$PREFIX|  <prefix>|" | sort
