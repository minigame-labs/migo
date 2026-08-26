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

# The cdylib is its own crate so that `platform` can stay an rlib (host builds
# and tests cannot link V8 into a shared object). Its
# `[lib] name` is `migo`, so cargo emits the shipping name directly.
# The package name and the directory name are deliberately two variables: every
# package carries the `migo-` prefix, while the directories do not repeat it.
# Deriving one from the other is what broke this script when the packages were
# renamed -- `pushd` into a non-existent directory left cargo running at the
# repository root, which has no manifest.
CRATE_NAME="migo-android-jni"
CRATE_DIR="crates/android-jni"

CRATE_SO_NAME="libmigo.so"
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

# shellcheck source=scripts/lib/android-ndk.sh
source "$SCRIPT_DIR/lib/android-ndk.sh"
# shellcheck source=scripts/lib/v8-materialise.sh
source "$SCRIPT_DIR/lib/v8-materialise.sh"

if [[ ! -d "$ENGINE_ROOT" ]]; then
    print_error "engine directory not found at $ENGINE_ROOT"
    exit 1
fi

# Checked here rather than at the `pushd` below, because a failed `pushd` does
# not stop the build: cargo simply runs in whatever directory the script was
# invoked from, and the error it reports is a missing manifest -- which reads
# like a broken checkout instead of a wrong path in this file.
if [[ ! -f "$CRATE_PATH/Cargo.toml" ]]; then
    print_error "crate manifest not found at $CRATE_PATH/Cargo.toml"
    print_error "CRATE_DIR ($CRATE_DIR) does not name a directory under $ENGINE_ROOT"
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
    echo "  ./build-android-so.sh [--product-profile full|slim] [--codegen-profile z|2|3] [--worker-snapshot] [--build-type release|debug] [--compile-only] [architectures...]"
    echo ""
    echo "Examples:"
    echo "  ./build-android-so.sh"
    echo "  ./build-android-so.sh arm64-v8a release"
    echo "  ./build-android-so.sh all --product-profile slim --codegen-profile 2 --build-type release"
    echo "  ./build-android-so.sh --compile-only arm64-v8a"
    echo ""
    echo "  --compile-only  compile the engine crates for the target and stop before"
    echo "                  the cdylib. This is what answers \"does this change compile"
    echo "                  for Android\" in about a minute warm, against the several"
    echo "                  minutes a full .so link costs. It builds migo-capi, which"
    echo "                  pulls core, graphics and platform -- the four crates whose"
    echo "                  only compile gate is the Android build. It does NOT cover"
    echo "                  the cdylib itself, so a change under crates/android-jni"
    echo "                  still needs a full build."
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

    android_ndk_read_pin "$PROJECT_ROOT/contracts/artifact-manifest/android-v8.lock.json" || exit 1
    android_ndk_resolve || { print_error "cannot resolve the pinned Android NDK"; exit 1; }

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
    local cargo_registry="${CARGO_HOME:-$HOME/.cargo}/registry/src"
    local skia_src
    # A missing registry is a supported cold-build state, and CARGO_HOME may be
    # isolated in CI. `find` must not trip `set -euo pipefail` before the
    # function can take its documented skip path.
    skia_src="$(find "$cargo_registry" -maxdepth 2 -type d -name 'skia-bindings-*' \
        2>/dev/null | sort | tail -1 || true)"
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
    local codegen_profile="$3"
    local worker_snapshot="$4"

    if [[ "$platform" == "armv7" || "$platform" == "x86" ]]; then
        print_error "$platform is not supported"
        return 1
    fi

    local target_triple="${PLATFORM_MAP[$platform]:-}"
    if [[ -z "$target_triple" ]]; then
        print_error "Unknown platform: $platform"
        return 1
    fi
    local -a profile_args=()
    local out_dir="debug"
    local destination_suffix=""

    if [[ "$build_type" == "release" ]]; then
        case "$codegen_profile" in
            z)
                profile_args=(--release)
                out_dir="release"
                ;;
            2)
                profile_args=(--profile release-hot2)
                out_dir="release-hot2"
                destination_suffix="-opt2"
                ;;
            3)
                profile_args=(--profile release-hot3)
                out_dir="release-hot3"
                destination_suffix="-opt3"
                ;;
        esac
    fi
    local cargo_features="profile-$product_profile"
    if [[ "$worker_snapshot" == true ]]; then
        destination_suffix+="-worker-snapshot"
        cargo_features+=",worker-snapshot"
    fi

    # --------------------------------------------------------
    # Rusty V8 config
    # --------------------------------------------------------
    local v8_dir="$V8_LIBS_DIR/$target_triple"

    # Verified and materialised under a path that is its own hash, rather than exported
    # from wherever the archive happens to sit. This used to be `[[ -f "$v8_archive" ]]`
    # -- existence, not identity -- so the AAR's native library was built against
    # whatever bytes were at that path, while the SDK scripts beside it already ran
    # verify-v8-component first. Content addressing also makes cargo's staleness rule
    # correct: it reruns the v8 build script when the *value* of RUSTY_V8_ARCHIVE changes,
    # not when the file does, so a rebuilt archive is a new path instead of a silent reuse.
    if ! v8_materialise "$v8_dir" "$PROJECT_ROOT/engine/target/v8-materialised"; then
        print_error "cannot use the V8 archive for $target_triple"
        exit 1
    fi
    export RUSTY_V8_ARCHIVE="$V8_MATERIALISED_ARCHIVE"
    export RUSTY_V8_SRC_BINDING_PATH="$V8_MATERIALISED_BINDING"
    print_info "RUSTY_V8_ARCHIVE = $RUSTY_V8_ARCHIVE"
    print_info "RUSTY_V8_SRC_BINDING_PATH = $RUSTY_V8_SRC_BINDING_PATH"

    # --------------------------------------------------------
    # RUSTFLAGS (arm64 builtins)
    # --------------------------------------------------------
    local orig_rustflags="${RUSTFLAGS:-}"

    # --allow-multiple-definition is needed on every Android target, and what it
    # tolerates was measured on 2026-08-10 by removing it: the link fails with exactly
    # six duplicate symbols, all of them ones libc++ explicitly instantiates in
    # stdexcept.cpp -- std::runtime_error and std::logic_error's char-const* and copy
    # constructors, and their copy assignments. lld names both providers:
    #
    #   V8's own stdexcept.o, from *Chromium's* libc++, inside
    #     target/<triple>/release/deps/.../libv8-*.rlib
    #   the NDK sysroot's libc++_static.a, at
    #     libcxx/src/support/runtime/stdexcept_default.ipp:35 of ndk-release-r23
    #
    # So the justification this comment used to carry -- "identical NDK ABI" -- was
    # wrong. These are two different libc++ implementations, and taking the first
    # definition is safe only because no std exception object crosses the V8/Skia
    # boundary: each side throws and catches internally, and the surface between them
    # is a C ABI. That is a narrower claim than "same ABI", and it is why item 1.4
    # stays open -- resolving this means one provider of those symbols, not a
    # friendlier linker.
    #
    # The flag only needs to exist on the final link of libmigo.so, which happens
    # inside the cargo invocation below.
    #
    # Do NOT add `-C embed-bitcode=no` here. The shipping [profile.release] uses
    # lto="fat", and rustc rejects `-C embed-bitcode=no` together with `-C lto`
    # as a hard error on the final cdylib. Under fat LTO cargo forces bitcode on
    # and the LTO step consumes every crate's bitcode (including the v8 binding
    # compiled from the prebuilt src_binding.rs) into native code before the
    # final link, so the NDK lld never sees a raw-bitcode object -- the failure
    # that once motivated embed-bitcode=no ("Invalid value Producer/Reader LLVM")
    # only happens without LTO, where cargo's default is already embed-bitcode=no.
    local common_rustflags="-C link-arg=-Wl,--allow-multiple-definition"

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
    print_info "Building $platform ($target_triple) [$build_type, codegen=$codegen_profile, worker-snapshot=$worker_snapshot]"

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

    pushd "$CRATE_PATH" > /dev/null || {
        print_error "cannot enter crate directory $CRATE_PATH"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    }

    # NB: `if !` / `||` around a function disables bash's `set -e` inside
    # it, which previously let a failed cargo build slip through because
    # a stale .so on disk made the later `cp` succeed.  Capture the exit
    # code explicitly so we can propagate the real failure upward.
    local cargo_rc=0
    local -a package_args=()
    local features_argument="$cargo_features"
    if [[ "$compile_only" == true ]]; then
        # `migo-capi` is the single package that pulls core, graphics and platform,
        # so selecting it compiles all four crates whose only compile gate is this
        # build. The feature is qualified because the package cargo would otherwise
        # apply a bare feature name to is this directory's cdylib, which is not
        # selected.
        package_args=(-p migo-capi)
        features_argument="migo-capi/profile-$product_profile"
    fi
    cargo ndk --target "$target_triple" --platform "$ANDROID_API" -- build \
        --target-dir "$TARGET_DIR" "${profile_args[@]}" "${package_args[@]}" \
        --no-default-features --features "$features_argument" \
        || cargo_rc=$?

    popd > /dev/null

    if [[ $cargo_rc -ne 0 ]]; then
        print_error "cargo build failed for $platform (rc=$cargo_rc)"
        export RUSTFLAGS="$orig_rustflags"
        return $cargo_rc
    fi

    if [[ "$compile_only" == true ]]; then
        export RUSTFLAGS="$orig_rustflags"
        print_success "Compiled core+graphics+platform+capi for $target_triple"
        return 0
    fi

    # --------------------------------------------------------
    # Copy output .so
    # --------------------------------------------------------
    local abi
    abi=$(get_abi_name "$platform")
    local dst_dir="$JNI_LIBS_DIR/${product_profile}${destination_suffix}/$abi"

    if ! mkdir -p "$dst_dir"; then
        print_error "Unable to create output directory: $dst_dir"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    fi

    local src_so="$TARGET_DIR/$target_triple/$out_dir/$CRATE_SO_NAME"
    local dst_so="$dst_dir/$OUTPUT_SO_NAME"

    if [[ ! -f "$src_so" ]]; then
        print_error "Output .so not found: $src_so"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    fi
    if ! cp "$src_so" "$dst_so"; then
        print_error "Unable to copy $src_so to $dst_so"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    fi
    print_success "Copied -> $dst_so"

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
        if ! cp "$libcpp_src" "$libcpp_dst"; then
            print_error "Unable to copy $libcpp_src to $libcpp_dst"
            export RUSTFLAGS="$orig_rustflags"
            return 1
        fi
        # Strip debug symbols from libc++_shared.so (NDK ships unstripped, ~6.6MB -> ~800KB)
        local llvm_strip_bin=""
        if command -v llvm-strip &>/dev/null; then
            llvm_strip_bin="$(command -v llvm-strip)"
        elif [[ -x "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/bin/llvm-strip" ]]; then
            llvm_strip_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/bin/llvm-strip"
        fi

        if [[ -n "$llvm_strip_bin" ]]; then
            if ! "$llvm_strip_bin" --strip-all "$libcpp_dst"; then
                print_error "Unable to strip $libcpp_dst"
                export RUSTFLAGS="$orig_rustflags"
                return 1
            fi
            print_success "Copied + stripped -> $libcpp_dst"
        else
            print_success "Copied -> $libcpp_dst (llvm-strip not found, skipped stripping)"
        fi
    else
        print_error "libc++_shared.so not found: $libcpp_src"
        export RUSTFLAGS="$orig_rustflags"
        return 1
    fi

    export RUSTFLAGS="$orig_rustflags"
    return 0
}

# ------------------------------------------------------------
# Main
# ------------------------------------------------------------
# Default to release.  The debug profile produces a ~347 MB .so (no
# strip / LTO / opt) versus ~49 MB for release; shipping debug into
# jniLibs was the single biggest size regression.  Pass `debug`
# explicitly when you need an unoptimized build for local debugging.
build_type="release"
product_profile="full"
codegen_profile="z"
worker_snapshot=false
compile_only=false
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
        --product-profile)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--product-profile requires a value"
                exit 1
            fi
            product_profile="$1"
            ;;
        --product-profile=*)
            product_profile="${arg#*=}"
            ;;
        --codegen-profile)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--codegen-profile requires a value"
                exit 1
            fi
            codegen_profile="$1"
            ;;
        --codegen-profile=*)
            codegen_profile="${arg#*=}"
            ;;
        --worker-snapshot)
            worker_snapshot=true
            ;;
        --compile-only)
            compile_only=true
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
if [[ "$product_profile" != "full" && "$product_profile" != "slim" ]]; then
    print_error "Invalid product profile: $product_profile (expected full|slim)"
    exit 1
fi
if [[ "$codegen_profile" != "z" && "$codegen_profile" != "2" && "$codegen_profile" != "3" ]]; then
    print_error "Invalid codegen profile: $codegen_profile (expected z|2|3)"
    exit 1
fi
if [[ "$build_type" == "debug" && "$codegen_profile" != "z" ]]; then
    print_error "Codegen profile $codegen_profile requires a release build"
    exit 1
fi
if [[ "$worker_snapshot" == true && ( "$build_type" != "release" || "$product_profile" != "full" ) ]]; then
    print_error "Worker snapshot requires a full release build"
    exit 1
fi
# Rejected rather than ignored: the worker snapshot is embedded by the cdylib this
# mode deliberately does not build, so accepting both would report a compile that
# never covered the requested configuration.
if [[ "$compile_only" == true && "$worker_snapshot" == true ]]; then
    print_error "--compile-only does not build the cdylib, so it cannot honour --worker-snapshot"
    exit 1
fi

check_dependencies

if [[ "$use_all" == true ]]; then
    platforms=("arm64-v8a" "x86_64")
fi

if [[ ${#platforms[@]} -eq 0 ]]; then
    platforms=("arm64-v8a" "x86_64")
    print_info "No platform specified, building default ABIs"
fi

print_info "Build type : $build_type"
print_info "Product    : $product_profile"
print_info "Codegen    : $codegen_profile"
print_info "Worker snap: $worker_snapshot"
print_info "Mode       : $([[ "$compile_only" == true ]] && echo "compile-only (no cdylib)" || echo "full .so")"
print_info "Platforms  : ${platforms[*]}"

# Stage-2: ensure the embedded ICU blob is the slim variant before building.
slim_icu_data

failed=()
for p in "${platforms[@]}"; do
    if ! build_platform "$p" "$build_type" "$codegen_profile" "$worker_snapshot"; then
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
