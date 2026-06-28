#!/usr/bin/env bash
# ============================================================
# Android dynamic library build script
# Location: scripts/build-android-so.sh
#
# Usage:
#   ./build-android-so.sh
#   ./build-android-so.sh arm64-v8a
#   ./build-android-so.sh arm64-v8a release
#   ./build-android-so.sh all release
# ============================================================

set -euo pipefail

# ------------------------------------------------------------
# Constants
# ------------------------------------------------------------
# Android minimum supported API level.
#
# Raised from 21 to 26 because skia-bindings 0.93 hard-codes API 26 in
# `build_support/platform/android.rs` (the first Oreo API; needed for
# full Vulkan and a number of modern NDK headers Skia depends on).
# Linking Skia against an older runtime would be ABI-unsafe, so we
# promote minSdk for the whole engine.
#
# In practice this is a non-issue: Google Play's April-2026 distribution
# report shows <1.5% of active Android devices below API 26.
ANDROID_API=26

CRATE_NAME="platform"
CRATE_DIR="crates/$CRATE_NAME"

# cargo output naming rule: lib{crate}.so
CRATE_SO_NAME="lib$CRATE_NAME.so"
OUTPUT_SO_NAME="libmigo.so"

declare -A PLATFORM_MAP=(
    ["arm64-v8a"]="aarch64-linux-android"
    ["x86_64"]="x86_64-linux-android"
)

# ------------------------------------------------------------
# Logging helpers
# ------------------------------------------------------------
print_info()    { echo -e "\033[0;36m[INFO] $1\033[0m"; }
print_success() { echo -e "\033[0;32m[SUCCESS] $1\033[0m"; }
print_warning() { echo -e "\033[0;33m[WARNING] $1\033[0m"; }
print_error()   { echo -e "\033[0;31m[ERROR] $1\033[0m"; }

# ------------------------------------------------------------
# Resolve paths
# ------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
CRATE_PATH="$ENGINE_ROOT/$CRATE_DIR"
TARGET_DIR="$ENGINE_ROOT/target"
JNI_LIBS_DIR="$ENGINE_ROOT/jniLibs"
V8_LIBS_DIR="$ENGINE_ROOT/third_party/rusty_v8"

if [[ ! -d "$ENGINE_ROOT" ]]; then
    print_error "engine directory not found at $ENGINE_ROOT"
    exit 1
fi

# ------------------------------------------------------------
# Help
# ------------------------------------------------------------
show_help() {
    echo "Android .so Builder"
    echo ""
    echo "Usage:"
    echo "  ./build-android-so.sh [arm64-v8a|x86_64|all] [release|debug]"
    echo "  ./build-android-so.sh [--build-type release|debug] [architectures...]"
    echo ""
    echo "Examples:"
    echo "  ./build-android-so.sh"
    echo "  ./build-android-so.sh arm64-v8a release"
    echo "  ./build-android-so.sh all --build-type release"
    exit 0
}

# Fast path help (do not require environment/dependencies).
for arg in "$@"; do
    if [[ "$arg" == "--help" || "$arg" == "-h" ]]; then
        show_help
    fi
done

# ------------------------------------------------------------
# Dependency check
# ------------------------------------------------------------
check_dependencies() {
    print_info "Checking dependencies..."

    if ! command -v cargo &> /dev/null; then
        print_error "cargo not found"
        exit 1
    fi

    if ! command -v cargo-ndk &> /dev/null; then
        print_error "cargo-ndk not found (install with: cargo install cargo-ndk)"
        exit 1
    fi

    if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
        print_error "ANDROID_NDK_HOME is not set"
        exit 1
    fi

    # skia-bindings' build script reads ANDROID_NDK (not _HOME).  Keep
    # them in sync so Skia cross-compile picks the same toolchain as
    # cargo-ndk.
    export ANDROID_NDK="$ANDROID_NDK_HOME"

    print_success "All dependencies are ready (ANDROID_NDK=$ANDROID_NDK)"
}

# ------------------------------------------------------------
# Get ABI name
# ------------------------------------------------------------
get_abi_name() {
    local platform="$1"
    case "$platform" in
        arm64-v8a) echo "arm64-v8a" ;;
        x86_64)    echo "x86_64" ;;
        *)         echo "unknown" ;;
    esac
}

# ------------------------------------------------------------
# Find arm64 clang builtins
# ------------------------------------------------------------
find_arm64_builtins() {
    local prebuilt="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt"
    if [[ ! -d "$prebuilt" ]]; then
        return 1
    fi

    find "$prebuilt" -name "libclang_rt.builtins-aarch64-android.a" 2>/dev/null | head -1
}

# ------------------------------------------------------------
# Detect host platform
# ------------------------------------------------------------
get_host_platform() {
    local prebuilt_root="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt"
    local candidates=()
    local uname_s
    local uname_m

    uname_s="$(uname -s)"
    uname_m="$(uname -m)"

    case "$uname_s" in
        Linux*)
            candidates+=("linux-$uname_m")
            candidates+=("linux-x86_64")
            candidates+=("linux-aarch64")
            ;;
        Darwin*)
            candidates+=("darwin-$uname_m")
            candidates+=("darwin-arm64")
            candidates+=("darwin-x86_64")
            ;;
        *)
            candidates+=("linux-x86_64")
            ;;
    esac

    local c
    for c in "${candidates[@]}"; do
        if [[ -d "$prebuilt_root/$c" ]]; then
            echo "$c"
            return 0
        fi
    done

    # fallback: pick first existing prebuilt directory
    if compgen -G "$prebuilt_root/*" > /dev/null; then
        basename "$(ls -d "$prebuilt_root"/* | head -1)"
        return 0
    fi

    return 1
}

# ------------------------------------------------------------
# Build one platform
# ------------------------------------------------------------
# ------------------------------------------------------------
# Slim the embedded ICU data (stage 2 size reduction)
# ------------------------------------------------------------
# Skia bakes an ICU data blob into libicu.a via GN's make_data_assembly
# (externals/icu/<variant>/icudtl.dat -> icudtl_dat.S -> libicu.a). On
# Android it picks the `android` variant (8.5 MB, 121 locales). Our game
# runtime never uses ICU locale/format data (no Intl); text rendering
# only needs the break-iterator + char/line data. The `flutter` variant
# (761 KB) keeps char.brk / line_normal_cj.brk / word.brk + SE-Asian
# dictionaries (thai/khmer/lao/burmese) and drops the locale tables,
# saving ~7.7 MB off the final .so.
#
# We overwrite the `android` variant file with the `flutter` one (idempotent;
# the original is kept as .orig). Done before cargo build so GN/ninja
# regenerate icudtl_dat.S from the smaller blob. Set MIGO_FULL_ICU=1 to
# restore the full android data.
slim_icu_data() {
    local skia_src
    skia_src="$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name 'skia-bindings-*' 2>/dev/null | sort | tail -1)"
    if [[ -z "$skia_src" ]]; then
        print_warning "skia-bindings src not found; skipping ICU slim"
        return 0
    fi
    local icu_dir="$skia_src/skia/third_party/externals/icu"
    local android_dat="$icu_dir/android/icudtl.dat"
    local flutter_dat="$icu_dir/flutter/icudtl.dat"
    [[ -f "$android_dat" && -f "$flutter_dat" ]] || { print_warning "ICU variants missing; skipping"; return 0; }

    # Keep a one-time backup of the ORIGINAL (large) android blob. Guard
    # against backing up an already-slimmed file: only snapshot when no
    # backup exists AND the current android blob differs from flutter
    # (i.e. it is still the original large variant).
    if [[ ! -f "$android_dat.orig" ]] && ! cmp -s "$flutter_dat" "$android_dat"; then
        cp "$android_dat" "$android_dat.orig"
    fi

    if [[ "${MIGO_FULL_ICU:-0}" == "1" ]]; then
        cp "$android_dat.orig" "$android_dat"
        print_info "ICU: restored full android data (MIGO_FULL_ICU=1)"
        return 0
    fi

    # overwrite only if not already the flutter (small) blob
    if ! cmp -s "$flutter_dat" "$android_dat"; then
        cp "$flutter_dat" "$android_dat"
        print_info "ICU: slimmed android->flutter icudtl ($(du -h "$android_dat" | cut -f1))"
        # force GN to regenerate the data assembly on next build
        find "$TARGET_DIR" -path '*skia-bindings*/out/skia/gen/third_party/icu/icudtl_dat.S' -delete 2>/dev/null || true
        find "$TARGET_DIR" -path '*skia-bindings*/out/skia/libicu.a' -delete 2>/dev/null || true
    else
        print_info "ICU: already slimmed (flutter blob)"
    fi
}

build_platform() {
    local platform="$1"
    local build_type="$2"

    if [[ "$platform" == "armv7" || "$platform" == "x86" ]]; then
        print_error "$platform is not supported"
        return 1
    fi

    local target_triple="${PLATFORM_MAP[$platform]:-}"
    if [[ -z "$target_triple" ]]; then
        print_error "Unknown platform: $platform"
        return 1
    fi
    local profile_flag=""
    local out_dir="debug"

    if [[ "$build_type" == "release" ]]; then
        profile_flag="--release"
        out_dir="release"
    fi

    # --------------------------------------------------------
    # Rusty V8 config
    # --------------------------------------------------------
    local arch
    if [[ "$platform" == "arm64-v8a" ]]; then
        arch="aarch64"
    else
        arch="x86_64"
    fi

    local v8_archive="$V8_LIBS_DIR/$arch/librusty_v8.a"
    local v8_binding="$V8_LIBS_DIR/$arch/src_binding.rs"

    if [[ -f "$v8_archive" ]]; then
        export RUSTY_V8_ARCHIVE="$v8_archive"
        print_info "RUSTY_V8_ARCHIVE = $v8_archive"
    fi

    if [[ -f "$v8_binding" ]]; then
        export RUSTY_V8_SRC_BINDING_PATH="$v8_binding"
        print_info "RUSTY_V8_SRC_BINDING_PATH = $v8_binding"
    fi

    # --------------------------------------------------------
    # RUSTFLAGS (arm64 builtins)
    # --------------------------------------------------------
    local orig_rustflags="${RUSTFLAGS:-}"

    # --allow-multiple-definition is needed on every Android target:
    # rusty_v8's prebuilt librusty_v8.a embeds parts of libc++, and
    # skia-bindings emits `-lc++_static`.  Both archives define
    # std::runtime_error / std::exception / ... → linker picks the first
    # occurrence, which is safe (identical NDK ABI) but lld defaults to
    # erroring.  The flag only needs to exist on the final link of
    # libmigo.so, which happens inside the cargo invocation below.
    # `embed-bitcode=no` must be repeated here: exporting RUSTFLAGS below
    # OVERRIDES (does not merge with) config.toml's [target] rustflags, so the
    # config's embed-bitcode=no would be dropped — and a fresh v8 crate build
    # then emits an LLVM-bitcode binding.o that the NDK lld cannot link
    # ("Invalid value (Producer: 'LLVM..' Reader: 'LLVM..')"). Normally hidden
    # because the v8 crate is cached; surfaces after `cargo clean -p v8`.
    local common_rustflags="-C link-arg=-Wl,--allow-multiple-definition -C embed-bitcode=no"

    if [[ "$platform" == "arm64-v8a" ]]; then
        local builtins
        builtins=$(find_arm64_builtins)
        if [[ -z "$builtins" ]]; then
            print_error "libclang_rt.builtins-aarch64-android.a not found"
            return 1
        fi

        local builtins_dir
        builtins_dir=$(dirname "$builtins")
        # --exclude-libs,ALL prevents re-exporting symbols from static libs
        # (e.g. V8, ring, Skia), reducing .dynsym/.rela.dyn by ~430KB.
        export RUSTFLAGS="$orig_rustflags $common_rustflags -L $builtins_dir -l static=clang_rt.builtins-aarch64-android -C link-arg=-Wl,--exclude-libs,ALL"
        print_info "Using arm64 clang builtins + --exclude-libs,ALL + --allow-multiple-definition"
    else
        export RUSTFLAGS="$orig_rustflags $common_rustflags"
    fi

    # --------------------------------------------------------
    # Build
    # --------------------------------------------------------
    print_info "Building $platform ($target_triple) [$build_type]"

    # Trim the bundled SQLite amalgamation to just the KV surface we
    # need. libsqlite3-sys's build.rs already injects a default set
    # of -DSQLITE_ENABLE_* (FTS3/FTS5/RTREE/JSON1/COLUMN_METADATA/…)
    # that the Rust API relies on; we can't override those without
    # forking. Everything below is a pure *subtraction* on top of
    # that default — each flag has to be confirmed non-conflicting
    # with the ENABLE_* set or the build fails at compile.
    #
    # `SQLITE_DQS=0` rejects MySQL-style double-quoted string
    # literals (forces "foo" to mean identifier, not string), the
    # modern recommended default.
    # `SQLITE_LIKE_DOESNT_MATCH_BLOBS` tightens LIKE semantics and
    # removes one corner-case code path we never use.
    local sqlite_omit_flags=(
        -DSQLITE_OMIT_LOAD_EXTENSION   # we never call .load_extension
        -DSQLITE_OMIT_DEPRECATED
        -DSQLITE_OMIT_AUTHORIZATION
        -DSQLITE_OMIT_SHARED_CACHE     # we use one Connection per session
        -DSQLITE_DQS=0
        -DSQLITE_DEFAULT_MEMSTATUS=0   # skip internal memory accounting
        -DSQLITE_LIKE_DOESNT_MATCH_BLOBS
        -DSQLITE_MAX_EXPR_DEPTH=0      # disables the parser's depth limiter
    )
    # libsqlite3-sys reads this env var and passes it straight to the
    # amalgamation compile.  Space-separated, no quoting needed.
    export LIBSQLITE3_FLAGS="${sqlite_omit_flags[*]}"

    pushd "$CRATE_PATH" > /dev/null

    # NB: `if !` / `||` around a function disables bash's `set -e` inside
    # it, which previously let a failed cargo build slip through because
    # a stale .so on disk made the later `cp` succeed.  Capture the exit
    # code explicitly so we can propagate the real failure upward.
    local cargo_rc=0
    cargo ndk --target "$target_triple" --platform "$ANDROID_API" -- build $profile_flag \
        || cargo_rc=$?

    popd > /dev/null

    if [[ $cargo_rc -ne 0 ]]; then
        print_error "cargo build failed for $platform (rc=$cargo_rc)"
        export RUSTFLAGS="$orig_rustflags"
        return $cargo_rc
    fi

    # --------------------------------------------------------
    # Copy output .so
    # --------------------------------------------------------
    local abi
    abi=$(get_abi_name "$platform")
    local dst_dir="$JNI_LIBS_DIR/$abi"

    mkdir -p "$dst_dir"

    local src_so="$TARGET_DIR/$target_triple/$out_dir/$CRATE_SO_NAME"
    local dst_so="$dst_dir/$OUTPUT_SO_NAME"

    if [[ -f "$src_so" ]]; then
        cp "$src_so" "$dst_so"
        print_success "Copied -> $dst_so"
    else
        print_warning "Output .so not found: $src_so"
    fi

    # --------------------------------------------------------
    # Copy libc++_shared.so (required by cpal/oboe shared STL)
    # Note: cannot use static STL because V8's prebuilt librusty_v8.a
    # already embeds static libc++ symbols — linking both causes duplicates.
    # --------------------------------------------------------
    local host_platform
    if ! host_platform=$(get_host_platform); then
        print_warning "Unable to detect NDK host platform under: $ANDROID_NDK_HOME/toolchains/llvm/prebuilt"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    fi
    local libcpp_src="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/sysroot/usr/lib/$target_triple/libc++_shared.so"
    local libcpp_dst="$dst_dir/libc++_shared.so"

    if [[ -f "$libcpp_src" ]]; then
        cp "$libcpp_src" "$libcpp_dst"
        # Strip debug symbols from libc++_shared.so (NDK ships unstripped, ~6.6MB -> ~800KB)
        local llvm_strip_bin=""
        if command -v llvm-strip &>/dev/null; then
            llvm_strip_bin="$(command -v llvm-strip)"
        elif [[ -x "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/bin/llvm-strip" ]]; then
            llvm_strip_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/bin/llvm-strip"
        fi

        if [[ -n "$llvm_strip_bin" ]]; then
            "$llvm_strip_bin" --strip-all "$libcpp_dst"
            print_success "Copied + stripped -> $libcpp_dst"
        else
            print_success "Copied -> $libcpp_dst (llvm-strip not found, skipped stripping)"
        fi
    else
        print_warning "libc++_shared.so not found: $libcpp_src"
    fi

    export RUSTFLAGS="$orig_rustflags"
    return 0
}

# ------------------------------------------------------------
# Main
# ------------------------------------------------------------
check_dependencies

# Default to release.  The debug profile produces a ~347 MB .so (no
# strip / LTO / opt) versus ~49 MB for release; shipping debug into
# jniLibs was the single biggest size regression.  Pass `debug`
# explicitly when you need an unoptimized build for local debugging.
build_type="release"
platforms=()
use_all=false

while [[ $# -gt 0 ]]; do
    arg="$1"
    case "$arg" in
        --help|-h)
            show_help
            ;;
        --build-type)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--build-type requires a value"
                exit 1
            fi
            build_type="$1"
            ;;
        --build-type=*)
            build_type="${arg#*=}"
            ;;
        release)
            build_type="release"
            ;;
        debug)
            build_type="debug"
            ;;
        all)
            use_all=true
            ;;
        arm64-v8a|x86_64)
            platforms+=("$arg")
            ;;
        *)
            print_error "Unknown argument: $arg"
            exit 1
            ;;
    esac
    shift
done

if [[ "$build_type" != "release" && "$build_type" != "debug" ]]; then
    print_error "Invalid build type: $build_type (expected release|debug)"
    exit 1
fi

if [[ "$use_all" == true ]]; then
    platforms=("arm64-v8a" "x86_64")
fi

if [[ ${#platforms[@]} -eq 0 ]]; then
    platforms=("arm64-v8a" "x86_64")
    print_info "No platform specified, building default ABIs"
fi

print_info "Build type : $build_type"
print_info "Platforms  : ${platforms[*]}"

# Stage-2: ensure the embedded ICU blob is the slim variant before building.
slim_icu_data

failed=()
for p in "${platforms[@]}"; do
    if ! build_platform "$p" "$build_type"; then
        failed+=("$p")
    fi
done

if [[ ${#failed[@]} -eq 0 ]]; then
    print_success "All Android builds succeeded"
    exit 0
else
    print_error "Failed platforms: ${failed[*]}"
    exit 1
fi
