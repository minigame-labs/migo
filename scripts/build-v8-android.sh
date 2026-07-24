#!/usr/bin/env bash
# ============================================================
# 从源码裁剪编译 rusty_v8 的 Android 静态库 (librusty_v8.a)
# Location: scripts/build-v8-android.sh
#
# 站在 termux-packages 的 patch 之上(允许自定义 NDK / sysroot /
# libcxx / jumbo build),但 GN_ARGS 完全可控并追加体积优化。
#
# 产出:engine/third_party/rusty_v8/<arch>/librusty_v8.a + src_binding.rs
#      + component-manifest.json (verified source/toolchain/GN provenance)
#       (覆盖现有预编译 archive)
#
# 前置:
#   - RUSTY_V8_SRC 指向已 clone + submodule 完成的 rusty_v8 源码树
#     (默认 /home/wkspace/rusty_v8_src)
#   - ANDROID_NDK_HOME 指向 NDK
#   - patch 目录:engine/third_party/v8-patches/
#
# 用法:
#   ./scripts/build-v8-android.sh aarch64               # official_build 优化版
#   ./scripts/build-v8-android.sh aarch64 --reproduce   # 先复现(不加 official_build)
#   V8_KEEP_I18N=1 ./scripts/build-v8-android.sh aarch64 # 保留 i18n(默认关闭)
# ============================================================
set -euo pipefail

# ------------------------------------------------------------
# Paths
# ------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
PATCH_DIR="$ENGINE_ROOT/third_party/v8-patches"
V8_OUT_DIR="$ENGINE_ROOT/third_party/rusty_v8"
V8_COMPONENT_WRITER="$SCRIPT_DIR/write-v8-component-manifest.py"
V8_BUILD_LOCK="$PROJECT_ROOT/contracts/artifact-manifest/android-v8.lock.json"

RUSTY_V8_SRC="${RUSTY_V8_SRC:-/home/wkspace/rusty_v8_src}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"
ANDROID_API="${ANDROID_API:-26}"

# ------------------------------------------------------------
# Logging
# ------------------------------------------------------------
info()  { echo -e "\033[0;36m[V8] $1\033[0m"; }
ok()    { echo -e "\033[0;32m[V8-OK] $1\033[0m"; }
warn()  { echo -e "\033[0;33m[V8-WARN] $1\033[0m"; }
err()   { echo -e "\033[0;31m[V8-ERR] $1\033[0m"; }

# ------------------------------------------------------------
# Args
# ------------------------------------------------------------
ARCH="aarch64"
REPRODUCE=false
for a in "$@"; do
    case "$a" in
        aarch64|x86_64) ARCH="$a" ;;
        --reproduce)    REPRODUCE=true ;;
        *) err "unknown arg: $a"; exit 1 ;;
    esac
done

case "$ARCH" in
    aarch64) TARGET="aarch64-linux-android"; GN_CPU="arm64" ;;
    x86_64)  TARGET="x86_64-linux-android";  GN_CPU="x64" ;;
esac

# ------------------------------------------------------------
# Sanity checks
# ------------------------------------------------------------
[[ -d "$RUSTY_V8_SRC" ]]        || { err "RUSTY_V8_SRC not found: $RUSTY_V8_SRC"; exit 1; }
[[ -f "$RUSTY_V8_SRC/build.rs" ]] || { err "not a rusty_v8 tree (no build.rs): $RUSTY_V8_SRC"; exit 1; }
[[ -d "$ANDROID_NDK_HOME" ]]   || { err "ANDROID_NDK_HOME not found: $ANDROID_NDK_HOME"; exit 1; }
[[ -d "$PATCH_DIR" ]]          || { err "patch dir not found: $PATCH_DIR"; exit 1; }
[[ -f "$V8_COMPONENT_WRITER" ]] || { err "component writer not found: $V8_COMPONENT_WRITER"; exit 1; }
[[ -f "$V8_BUILD_LOCK" ]]       || { err "V8 source lock not found: $V8_BUILD_LOCK"; exit 1; }
[[ "$ANDROID_API" == "26" ]]    || { err "verified Android V8 builds require ANDROID_API=26"; exit 1; }

# submodule completeness
if [[ ! -f "$RUSTY_V8_SRC/v8/include/v8-version.h" ]]; then
    err "v8 submodule not checked out yet (no v8/include/v8-version.h)"
    err "run: cd $RUSTY_V8_SRC && git submodule update --init --recursive"
    exit 1
fi

V8_VER="$(awk '/#define V8_MAJOR_VERSION/{a=$3} /#define V8_MINOR_VERSION/{b=$3} /#define V8_BUILD_NUMBER/{c=$3} /#define V8_PATCH_LEVEL/{d=$3} END{print a"."b"."c"."d}' "$RUSTY_V8_SRC/v8/include/v8-version.h")"
info "V8 version: $V8_VER  (target=$TARGET, api=$ANDROID_API, reproduce=$REPRODUCE)"

# ------------------------------------------------------------
# Apply termux patches (idempotent via a stamp file)
# ------------------------------------------------------------
# All 7 patches apply at the rusty_v8 root with -p1.  The search-files
# diff is for deno's vendored layout only and is NOT applied here.
PATCH_STAMP="$RUSTY_V8_SRC/.migo_patches_applied"

apply_patches() {
    info "ensuring termux v8 patches are applied"
    # Only the 3 patches required for correct Android cross-compile of
    # V8 14.5.201:
    #   0001 run_bindgen.py  - strip BINDGEN_EXTRA_CLANG_ARGS* (bindgen)
    #   0002 build.rs        - use_sysroot on android (cross-compile)
    #   0003 c++.gni         - custom libcxx for v8 snapshot toolchain
    # Deliberately skipped:
    #   0004 allow-custom-ndk - already upstreamed in V8 14.5
    #   0101/0102/0103 jumbo  - build-speed only, hunks don't align on
    #     14.5.201 (regexp/sandbox/v8.gni drift)
    #
    # We verify each patch by a SENTINEL string in the target file rather
    # than trusting a stamp: gn/submodule operations during the build can
    # reset build/ and silently drop the patch. (target_file, sentinel,
    # patch_glob) triples:
    local specs=(
        "build/rust/gni_impl/run_bindgen.py|BINDGEN_EXTRA_CLANG_ARGS|0001-*.patch"
        "build.rs|target_os == \"linux\" || target_os == \"android\"|0002-*.patch"
        "build/config/c++/c++.gni|snapshot_toolchain.gni|0003-*.patch"
    )
    local spec
    for spec in "${specs[@]}"; do
        local tgt="${spec%%|*}"; local rest="${spec#*|}"
        local sentinel="${rest%%|*}"; local glob="${rest##*|}"
        local -a matches=("$PATCH_DIR"/$glob)
        local pf="${matches[0]}"
        [[ -f "$pf" ]] || { err "missing patch: $glob"; exit 1; }
        if grep -qF "$sentinel" "$RUSTY_V8_SRC/$tgt" 2>/dev/null; then
            echo "  = already in effect: $(basename "$pf")"
        else
            # No `</dev/null`: a second redirect on the same descriptor wins, so
            # it fed patch an empty stdin -- patch then exited 0 having applied
            # nothing. It went unnoticed because this tree normally already
            # carries the patches as uncommitted changes, so the sentinel check
            # above takes the "already in effect" branch and this line never
            # runs. `--batch` already suppresses the prompting it guarded against.
            if patch -p1 -d "$RUSTY_V8_SRC" --batch --forward < "$pf"; then
                # confirm it actually took
                if grep -qF "$sentinel" "$RUSTY_V8_SRC/$tgt" 2>/dev/null; then
                    echo "  ✓ applied $(basename "$pf")"
                else
                    err "patch ran but sentinel missing in $tgt: $(basename "$pf")"
                    exit 1
                fi
            else
                err "patch failed: $(basename "$pf")"
                exit 1
            fi
        fi
    done
    ok "patches verified (jumbo skipped)"
}

apply_patches

# ------------------------------------------------------------
# NDK wiring: symlink user's NDK into rusty_v8's expected location
# so build.rs skips the r26c auto-download (it checks for
# third_party/android_ndk/.../aarch64-linux-android24-clang++).
# ------------------------------------------------------------
NDK_HOST="linux-x86_64"
LINK="$RUSTY_V8_SRC/third_party/android_ndk"
mkdir -p "$RUSTY_V8_SRC/third_party"
if [[ ! -e "$LINK" ]]; then
    ln -sf "$ANDROID_NDK_HOME" "$LINK"
    info "linked NDK -> third_party/android_ndk"
fi
if [[ ! -e "$LINK/toolchains/llvm/prebuilt/$NDK_HOST/bin/aarch64-linux-android24-clang++" ]]; then
    warn "NDK lacks android24-clang++; build.rs may try to download r26c"
fi

# ------------------------------------------------------------
# GN args
#   Base set mirrors termux's working Android recipe.
#   Size/perf policy (locked decisions):
#     - JIT (TurboFan/Maglev/Sparkplug): KEPT  (performance first)
#     - WebAssembly: KEPT  (games depend on it)
#     - pointer compression: KEPT  (memory)
#     - i18n: DISABLED  (no game uses Intl; drops V8-side ICU)
#     - is_official_build: ENABLED unless --reproduce (the main new win)
# ------------------------------------------------------------
NDK_VER="$(awk -F= '/^Pkg\.Revision/{gsub(/[[:space:]]/, "", $2); split($2, parts, "."); print parts[1]; exit}' \
    "$ANDROID_NDK_HOME/source.properties" 2>/dev/null || true)"
[[ -n "$NDK_VER" ]] || { err "cannot read NDK major revision from source.properties"; exit 1; }

GN_ARGS="android_ndk_api_level=$ANDROID_API"
GN_ARGS+=" android_ndk_root=\"$ANDROID_NDK_HOME\""
GN_ARGS+=" android_ndk_version=\"r${NDK_VER}\""
# (jumbo intentionally not set: we skip the jumbo patches on 14.5.201,
# and use_jumbo_build isn't a declared arg without them.)
GN_ARGS+=" v8_enable_webassembly=true"
GN_ARGS+=" v8_enable_pointer_compression=true"

# i18n: rusty_v8's src/binding.cc UNCONDITIONALLY includes
# <unicode/locid.h> and calls icu::Locale (exposes V8 locale API to
# Rust/deno_core). Disabling i18n removes the ICU headers and breaks the
# binding compile ("'unicode/locid.h' file not found"). So i18n stays ON
# by default; the ICU *data* (8.5M) is stripped separately in stage 2,
# which is where the real size win is. Set V8_NO_I18N=1 only if you also
# patch binding.cc to drop its ICU usage.
if [[ "${V8_NO_I18N:-0}" == "1" ]]; then
    GN_ARGS+=" v8_enable_i18n_support=false"
    warn "i18n: DISABLED (V8_NO_I18N=1) — requires patched binding.cc"
else
    GN_ARGS+=" v8_enable_i18n_support=true"
    info "i18n: ENABLED (required by rusty_v8 binding.cc)"
fi

if [[ "$REPRODUCE" == false ]]; then
    # The size win we did not have before.
    GN_ARGS+=" is_official_build=true"
    GN_ARGS+=" symbol_level=0"
    # is_official_build defaults chrome_pgo_phase=2, which downloads a
    # Chrome-specific PGO profile from a GS bucket (fails + irrelevant to
    # our V8 lib). Disable PGO; keep official_build's other size wins.
    GN_ARGS+=" chrome_pgo_phase=0"
    # NB: snapshot compression needs v8_use_zlib=true (extra dep); the
    # blob saving is small, so we skip it to avoid pulling zlib into V8.
    GN_ARGS+=" v8_enable_sandbox=false"
    # Drop C++ unwind tables (.eh_frame). Chromium keeps them on Android
    # even for official builds (exclude_unwind_tables defaults to
    # `is_official_build && !is_android` = false here) for crash
    # reporting. Our .so runs with panic=abort and never unwinds C++
    # exceptions, so the per-function CFI in .eh_frame is dead weight
    # (~1.6 MB of V8). This does NOT affect V8's JS-level unwinder API
    # (v8-unwinder.h), which is separate.
    GN_ARGS+=" exclude_unwind_tables=true"
    # ThinLTO OFF (measured 2026-07-14: no stress fps gain). The migo-vs-Chromium
    # stress gap is NOT V8 execution — pure-JS microbenches (mono/mega/GC) run
    # EQUAL-OR-FASTER on migo's V8 14.5 than on webview's V8 11.4. The gap is the
    # rendering/command-stream path, not V8, so PGO/ThinLTO don't help. Keeping
    # native objects → links with plain NDK lld 12, portable + reproducible.
    # (To re-experiment with ThinLTO: use_thin_lto=true + point migo's linker at
    # rusty_v8's own ld.lld 22 via engine/.cargo/config.toml --ld-path.)
    GN_ARGS+=" use_thin_lto=false"
    info "optimized build: +is_official_build +symbol_level=0 (pgo off, no unwind tables, no thin-lto)"
else
    info "reproduce build: termux-equivalent args only (no official_build)"
fi

export EXTRA_GN_ARGS="$GN_ARGS"
export V8_FROM_SOURCE=1
# NB: do NOT set CLANG_BASE_PATH to the NDK clang. That var is for the
# HOST toolchain (mksnapshot/torque run on the build machine); the NDK
# clang targets Android. termux leaves it to build.rs, which finds a
# system clang or downloads Chromium's. We do the same.

# GN + NINJA: build.rs auto-downloads these from chrome-infra-packages
# via ninja_gn_binaries.py, which uses http.client (does NOT honor the
# https_proxy env var) and times out here. We pre-fetch the *correct*
# gn (V8 14.5 needs gn >= 2315 for path_exists(); skia's bundled gn 2175
# is too old) into the source tree and point GN at it. Precedence:
#   V8_GN_PATH env > prefetched third_party/v8_correct_gn/gn > system gn
PREFETCHED_GN="$RUSTY_V8_SRC/third_party/v8_correct_gn/gn"
GN_BIN="${V8_GN_PATH:-}"
[[ -z "$GN_BIN" && -x "$PREFETCHED_GN" ]] && GN_BIN="$PREFETCHED_GN"
[[ -z "$GN_BIN" ]] && GN_BIN="$(command -v gn 2>/dev/null || true)"

if [[ -n "$GN_BIN" && -x "$GN_BIN" ]]; then
    export GN="$GN_BIN"
    info "using gn: $GN ($("$GN" --version 2>/dev/null))"
else
    err "no gn binary available and chrome-infra download is blocked."
    err "install gn, or set V8_GN_PATH=/path/to/gn"
    exit 1
fi

# ninja: prefer system, else error (we don't rely on the blocked download)
NINJA_BIN="$(command -v ninja 2>/dev/null || true)"
if [[ -n "$NINJA_BIN" ]]; then
    export NINJA="$NINJA_BIN"
    info "using ninja: $NINJA ($("$NINJA" --version 2>/dev/null))"
else
    err "no ninja binary; install ninja-build"
    exit 1
fi

info "EXTRA_GN_ARGS = $EXTRA_GN_ARGS"

# ------------------------------------------------------------
# Build via cargo (rusty_v8's build.rs drives gn + ninja)
# ------------------------------------------------------------
# LIBCLANG_PATH: rusty_v8's top-level bindgen (build.rs:233) parses V8's
# vendored libc++ headers, which require Clang 19+ (use __builtin_clzg/
# ctzg, "Libc++ only supports Clang 20 and later"). The system libclang
# is 18 (too old). NDK r28c ships libclang 19.0.1 — use it. Overridable
# via V8_LIBCLANG_PATH.
LIBCLANG_DIR="${V8_LIBCLANG_PATH:-}"
if [[ -z "$LIBCLANG_DIR" ]]; then
    # prefer an NDK that ships clang >= 19
    for cand in "$HOME/Android/android-ndk-r28c" "$HOME/Android/android-ndk-r29"; do
        if [[ -f "$cand/toolchains/llvm/prebuilt/$NDK_HOST/lib/libclang.so" ]]; then
            LIBCLANG_DIR="$cand/toolchains/llvm/prebuilt/$NDK_HOST/lib"
            break
        fi
    done
fi
if [[ -n "$LIBCLANG_DIR" && -f "$LIBCLANG_DIR/libclang.so" ]]; then
    export LIBCLANG_PATH="$LIBCLANG_DIR"
    info "LIBCLANG_PATH = $LIBCLANG_PATH (bindgen needs clang 19+)"
else
    warn "no clang19+ libclang found; bindgen may fail on V8's libc++ headers"
fi

# bindgen needs the NDK sysroot to find Android headers.
BINDGEN_SYSROOT="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST/sysroot"
export BINDGEN_EXTRA_CLANG_ARGS="--target=$TARGET --sysroot=$BINDGEN_SYSROOT"
TARGET_U="$(echo "$TARGET" | tr 'a-z-' 'A-Z_')"
export "BINDGEN_EXTRA_CLANG_ARGS_${TARGET_U}=$BINDGEN_EXTRA_CLANG_ARGS"

# Ensure the android target is installed.
rustup target add "$TARGET" >/dev/null 2>&1 || true

info "starting cargo build (first run downloads gn/sysroot/android_platform, slow)"
pushd "$RUSTY_V8_SRC" >/dev/null

BUILD_LOG="${TMPDIR:-/tmp}/migo-v8-build-${ARCH}.$$.log"
set +e
cargo build --release --target "$TARGET" -vv 2>&1 | tee "$BUILD_LOG" | \
    grep --line-buffered -iE "error|warning: |Compiling v8|Finished|gn gen|ninja|downloading|cloning" | tail -100
RC=${PIPESTATUS[0]}
set -e
popd >/dev/null

if [[ $RC -ne 0 ]]; then
    err "cargo build failed (rc=$RC). full log: $BUILD_LOG"
    err "last 20 lines:"
    tail -20 "$BUILD_LOG"
    exit $RC
fi

# ------------------------------------------------------------
# Locate + copy artifacts
# ------------------------------------------------------------
GN_OUT="$RUSTY_V8_SRC/target/$TARGET/release/gn_out/obj/librusty_v8.a"
[[ -f "$GN_OUT" ]] || GN_OUT="$(find "$RUSTY_V8_SRC/target/$TARGET/release" -name 'librusty_v8.a' -print -quit 2>/dev/null)"
BINDING="$(find "$RUSTY_V8_SRC/target/$TARGET/release" -name 'src_binding*.rs' -print -quit 2>/dev/null)"

[[ -f "$GN_OUT" ]] || { err "librusty_v8.a not found after build"; exit 1; }

DEST="$V8_OUT_DIR/$ARCH"
mkdir -p "$DEST"
# back up the existing (termux) archive once
if [[ -f "$DEST/librusty_v8.a" && ! -f "$DEST/librusty_v8.a.termux-bak" ]]; then
    cp "$DEST/librusty_v8.a" "$DEST/librusty_v8.a.termux-bak"
    info "backed up existing archive -> librusty_v8.a.termux-bak"
fi

cp "$GN_OUT" "$DEST/librusty_v8.a"
ok "archive -> $DEST/librusty_v8.a ($(ls -lh "$DEST/librusty_v8.a" | awk '{print $5}'))"
[[ -n "$BINDING" && -f "$BINDING" ]] || { err "src_binding.rs not found after build"; exit 1; }
cp "$BINDING" "$DEST/src_binding.rs"
ok "binding -> $DEST/src_binding.rs"

python3 "$V8_COMPONENT_WRITER" \
    --repo-root "$PROJECT_ROOT" \
    --rusty-v8-src "$RUSTY_V8_SRC" \
    --ndk-home "$ANDROID_NDK_HOME" \
    --arch "$ARCH" \
    --extra-gn-args "$EXTRA_GN_ARGS" \
    --archive "$DEST/librusty_v8.a" \
    --binding "$DEST/src_binding.rs" \
    --output "$DEST/component-manifest.json" \
    --lock "$V8_BUILD_LOCK"
ok "component manifest -> $DEST/component-manifest.json"

ok "V8 build complete for $ARCH (reproduce=$REPRODUCE)"
echo "next: rebuild libmigo.so via scripts/build-android-so.sh $([ "$ARCH" = aarch64 ] && echo arm64-v8a || echo x86_64)"
