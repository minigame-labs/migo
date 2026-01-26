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

set -e

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
    case "$(uname -s)" in
        Linux*)  echo "linux-x86_64" ;;
        Darwin*) echo "darwin-x86_64" ;;
        *)       echo "linux-x86_64" ;;
    esac
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

    local target_triple="${PLATFORM_MAP[$platform]}"
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
    host_platform=$(get_host_platform)
    local libcpp_src="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_platform/sysroot/usr/lib/$target_triple/libc++_shared.so"
    local libcpp_dst="$dst_dir/libc++_shared.so"

    if [[ -f "$libcpp_src" ]]; then
        cp "$libcpp_src" "$libcpp_dst"
        print_success "Copied -> $libcpp_dst"
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

for arg in "$@"; do
    if [[ "$arg" == "release" ]]; then
        build_type="release"
    elif [[ "$arg" == "all" ]]; then
        platforms=("arm64-v8a" "x86_64")
        break
    elif [[ -n "${PLATFORM_MAP[$arg]:-}" ]]; then
        platforms+=("$arg")
    fi
done

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
