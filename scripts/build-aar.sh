#!/usr/bin/env bash
# ============================================================
# MiniGame Android AAR Builder
# Location: scripts/build-aar.sh
#
# Usage:
#   ./build-aar.sh [release|debug] [architectures...]
#
# Examples:
#   ./build-aar.sh                    # release build, all architectures
#   ./build-aar.sh release            # release build, all architectures
#   ./build-aar.sh release arm64-v8a  # release build, arm64 only
# ============================================================

set -euo pipefail

# ------------------------------------------------------------
# Logging helpers
# ------------------------------------------------------------
print_info()    { echo -e "\033[0;36m[INFO] $1\033[0m"; }
print_success() { echo -e "\033[0;32m[SUCCESS] $1\033[0m"; }
print_warning() { echo -e "\033[0;33m[WARNING] $1\033[0m"; }
print_error()   { echo -e "\033[0;31m[ERROR] $1\033[0m"; }

# ------------------------------------------------------------
# Help
# ------------------------------------------------------------
show_help() {
    echo "MiniGame Android AAR Builder"
    echo ""
    echo "Usage:"
    echo "  ./build-aar.sh [release|debug] [architectures...]"
    echo "  ./build-aar.sh [--build-type release|debug] [--output-dir dist] [--skip-rust] [architectures...]"
    echo ""
    echo "Options:"
    echo "  release|debug    Build type (default: release)"
    echo "  --build-type     Build type (release|debug)"
    echo "  --output-dir     Output directory under platforms/android (default: dist)"
    echo "  arm64-v8a        Build for ARM64"
    echo "  x86_64           Build for x86_64"
    echo "  all              Build for all architectures"
    echo "  --skip-rust      Skip Rust build step"
    echo "  --help           Show this help"
    exit 0
}

# Fast path help (do not require environment/dependencies).
for arg in "$@"; do
    if [[ "$arg" == "--help" || "$arg" == "-h" ]]; then
        show_help
    fi
done

# ------------------------------------------------------------
# Path Resolution
# ------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ANDROID_DIR="$REPO_ROOT/platforms/android"
LIBRARY_DIR="$ANDROID_DIR/library"
RUST_BUILD_SCRIPT="$SCRIPT_DIR/build-android-so.sh"
EXTERNAL_JNI_LIBS="$REPO_ROOT/engine/jniLibs"
OUTPUT_DIR="dist"

echo "========================================"
echo "MiniGame Android AAR Builder"
echo "========================================"
print_info "RepoRoot:   $REPO_ROOT"
print_info "AndroidDir: $ANDROID_DIR"
print_info "LibraryDir: $LIBRARY_DIR"
echo ""

# ------------------------------------------------------------
# Sanity Checks
# ------------------------------------------------------------
if [[ ! -d "$ANDROID_DIR" ]]; then
    print_error "Android directory not found: $ANDROID_DIR"
    exit 1
fi

if [[ ! -d "$LIBRARY_DIR" ]]; then
    print_error "Android library module not found: $LIBRARY_DIR"
    exit 1
fi

if [[ ! -f "$RUST_BUILD_SCRIPT" ]]; then
    print_error "Rust build script not found: $RUST_BUILD_SCRIPT"
    exit 1
fi

# ------------------------------------------------------------
# Parse arguments
# ------------------------------------------------------------
BUILD_TYPE="release"
SKIP_RUST=false
ARCHITECTURES=()
USE_ALL_ARCH=false

while [[ $# -gt 0 ]]; do
    arg="$1"
    case "$arg" in
        --help|-h)
            show_help
            ;;
        --skip-rust)
            SKIP_RUST=true
            ;;
        --build-type)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--build-type requires a value"
                exit 1
            fi
            BUILD_TYPE="$1"
            ;;
        --build-type=*)
            BUILD_TYPE="${arg#*=}"
            ;;
        --output-dir)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--output-dir requires a value"
                exit 1
            fi
            OUTPUT_DIR="$1"
            ;;
        --output-dir=*)
            OUTPUT_DIR="${arg#*=}"
            ;;
        release)
            BUILD_TYPE="release"
            ;;
        debug)
            BUILD_TYPE="debug"
            ;;
        all)
            USE_ALL_ARCH=true
            ;;
        arm64-v8a|x86_64)
            ARCHITECTURES+=("$arg")
            ;;
        *)
            print_error "Unknown argument: $arg"
            exit 1
            ;;
    esac
    shift
done

if [[ "$BUILD_TYPE" != "release" && "$BUILD_TYPE" != "debug" ]]; then
    print_error "Invalid build type: $BUILD_TYPE (expected release|debug)"
    exit 1
fi

if [[ ${#ARCHITECTURES[@]} -eq 0 ]]; then
    ARCHITECTURES=("arm64-v8a" "x86_64")
fi

if [[ "$USE_ALL_ARCH" == true ]]; then
    ARCHITECTURES=("arm64-v8a" "x86_64")
fi

# ------------------------------------------------------------
# Build Rust (.so)
# ------------------------------------------------------------
build_rust_library() {
    if [[ "$SKIP_RUST" == true ]]; then
        print_info "Skipping Rust build"
        return
    fi

    print_info "Building Rust Android .so..."

    for arch in "${ARCHITECTURES[@]}"; do
        print_info "→ Rust build: $arch ($BUILD_TYPE)"
        "$RUST_BUILD_SCRIPT" "$arch" "$BUILD_TYPE"
    done

    print_success "Rust build done"
}

# ------------------------------------------------------------
# Copy JNI Libraries
# ------------------------------------------------------------
copy_native_libraries() {
    print_info "Copying JNI libraries..."

    local jni_libs_dir="$LIBRARY_DIR/src/main/jniLibs"

    if [[ -d "$jni_libs_dir" ]]; then
        rm -rf "$jni_libs_dir"
    fi
    mkdir -p "$jni_libs_dir"

    if [[ ! -d "$EXTERNAL_JNI_LIBS" ]]; then
        print_error "jniLibs directory not found: $EXTERNAL_JNI_LIBS"
        exit 1
    fi

    for arch in "${ARCHITECTURES[@]}"; do
        local src="$EXTERNAL_JNI_LIBS/$arch"
        local dst="$jni_libs_dir/$arch"

        if [[ ! -d "$src" ]]; then
            print_error "Missing native libs for $arch"
            exit 1
        fi

        mkdir -p "$dst"
        cp "$src"/* "$dst/"
        print_success "$arch copied"
    done
}

# ------------------------------------------------------------
# Build AAR
# ------------------------------------------------------------
build_aar() {
    print_info "Building AAR..."

    pushd "$ANDROID_DIR" > /dev/null

    local gradle_cmd
    if [[ -f "./gradlew" ]]; then
        chmod +x ./gradlew
        gradle_cmd="./gradlew"
    elif command -v gradle &> /dev/null; then
        gradle_cmd="gradle"
    else
        print_error "Gradle not found"
        exit 1
    fi

    $gradle_cmd clean

    if [[ "$BUILD_TYPE" == "release" ]]; then
        $gradle_cmd assembleRelease
    else
        $gradle_cmd assembleDebug
    fi

    popd > /dev/null

    print_success "AAR build success"
}

# ------------------------------------------------------------
# Collect Outputs
# ------------------------------------------------------------
collect_outputs() {
    print_info "Collecting outputs..."

    local out_dir="$ANDROID_DIR/$OUTPUT_DIR"

    if [[ -d "$out_dir" ]]; then
        rm -rf "$out_dir"
    fi
    mkdir -p "$out_dir"

    local aar_dir="$LIBRARY_DIR/build/outputs/aar"
    cp "$aar_dir"/* "$out_dir/"

    cat > "$out_dir/version.json" << EOF
{
    "buildType": "$BUILD_TYPE",
    "buildTime": "$(date '+%Y-%m-%d %H:%M:%S')"
}
EOF

    print_success "Outputs ready: $out_dir"
}

# ------------------------------------------------------------
# Main
# ------------------------------------------------------------
build_rust_library
copy_native_libraries
build_aar
collect_outputs

echo ""
echo "========================================"
print_success "Android AAR build completed"
echo "========================================"
