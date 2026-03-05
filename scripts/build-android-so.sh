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
ANDROID_API=21

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

    if [[ -z "$ANDROID_NDK_HOME" ]]; then
        print_error "ANDROID_NDK_HOME is not set"
        exit 1
    fi

    print_success "All dependencies are ready"
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

    if [[ "$platform" == "arm64-v8a" ]]; then
        local builtins
        builtins=$(find_arm64_builtins)
        if [[ -z "$builtins" ]]; then
            print_error "libclang_rt.builtins-aarch64-android.a not found"
            return 1
        fi

        local builtins_dir
        builtins_dir=$(dirname "$builtins")
        export RUSTFLAGS="$orig_rustflags -L $builtins_dir -l static=clang_rt.builtins-aarch64-android"
        print_info "Using arm64 clang builtins"
    fi

    # --------------------------------------------------------
    # Build
    # --------------------------------------------------------
    print_info "Building $platform ($target_triple) [$build_type]"

    pushd "$CRATE_PATH" > /dev/null

    cargo ndk --target "$target_triple" --platform "$ANDROID_API" -- build $profile_flag

    popd > /dev/null

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
    # Copy libc++_shared.so (required by cpal/oboe)
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

build_type="debug"
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
