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
    echo "  ./build-aar.sh [--product-profile full|slim] [--codegen-profile z|2|3] [--worker-snapshot] [--build-type release|debug] [--output-dir dist] [--skip-rust] [architectures...]"
    echo ""
    echo "Options:"
    echo "  release|debug    Build type (default: release)"
    echo "  --build-type     Build type (release|debug)"
    echo "  --product-profile Product surface (full|slim, default: full)"
    echo "  --codegen-profile Hot-crate optimization (z|2|3, default: z; 2/3 require release)"
    echo "  --worker-snapshot Build the isolated full-release Worker snapshot candidate"
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

if [[ ! -x "$RUST_BUILD_SCRIPT" ]]; then
    chmod +x "$RUST_BUILD_SCRIPT"
fi

# ------------------------------------------------------------
# Parse arguments
# ------------------------------------------------------------
BUILD_TYPE="release"
PRODUCT_PROFILE="full"
CODEGEN_PROFILE="z"
WORKER_SNAPSHOT=false
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
        --worker-snapshot)
            WORKER_SNAPSHOT=true
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
        --product-profile)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--product-profile requires a value"
                exit 1
            fi
            PRODUCT_PROFILE="$1"
            ;;
        --product-profile=*)
            PRODUCT_PROFILE="${arg#*=}"
            ;;
        --codegen-profile)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--codegen-profile requires a value"
                exit 1
            fi
            CODEGEN_PROFILE="$1"
            ;;
        --codegen-profile=*)
            CODEGEN_PROFILE="${arg#*=}"
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
if [[ "$PRODUCT_PROFILE" != "full" && "$PRODUCT_PROFILE" != "slim" ]]; then
    print_error "Invalid product profile: $PRODUCT_PROFILE (expected full|slim)"
    exit 1
fi
if [[ "$CODEGEN_PROFILE" != "z" && "$CODEGEN_PROFILE" != "2" && "$CODEGEN_PROFILE" != "3" ]]; then
    print_error "Invalid codegen profile: $CODEGEN_PROFILE (expected z|2|3)"
    exit 1
fi
if [[ "$BUILD_TYPE" == "debug" && "$CODEGEN_PROFILE" != "z" ]]; then
    print_error "Codegen profile $CODEGEN_PROFILE requires a release build"
    exit 1
fi
if [[ "$WORKER_SNAPSHOT" == true && ( "$BUILD_TYPE" != "release" || "$PRODUCT_PROFILE" != "full" ) ]]; then
    print_error "Worker snapshot requires a full release build"
    exit 1
fi

SOURCE_DATE_EPOCH_VALUE="${SOURCE_DATE_EPOCH:-}"
SOURCE_DATE_EPOCH_JSON="null"
if [[ -n "$SOURCE_DATE_EPOCH_VALUE" ]]; then
    if [[ ! "$SOURCE_DATE_EPOCH_VALUE" =~ ^[0-9]+$ ]]; then
        print_error "Invalid SOURCE_DATE_EPOCH: expected non-negative Unix seconds"
        exit 1
    fi
    SOURCE_DATE_EPOCH_JSON="\"$SOURCE_DATE_EPOCH_VALUE\""
fi

CODEGEN_SUFFIX=""
CARGO_PROFILE="debug"
if [[ "$BUILD_TYPE" == "release" ]]; then
    case "$CODEGEN_PROFILE" in
        z) CARGO_PROFILE="release" ;;
        2)
            CARGO_PROFILE="release-hot2"
            CODEGEN_SUFFIX="-opt2"
            ;;
        3)
            CARGO_PROFILE="release-hot3"
            CODEGEN_SUFFIX="-opt3"
            ;;
    esac
fi
WORKER_SNAPSHOT_SUFFIX=""
WORKER_SNAPSHOT_ARGS=()
if [[ "$WORKER_SNAPSHOT" == true ]]; then
    WORKER_SNAPSHOT_SUFFIX="-worker-snapshot"
    WORKER_SNAPSHOT_ARGS=(--worker-snapshot)
fi
ARTIFACT_SUFFIX="${CODEGEN_SUFFIX}${WORKER_SNAPSHOT_SUFFIX}"
EXTERNAL_JNI_LIBS="$REPO_ROOT/engine/jniLibs/${PRODUCT_PROFILE}${ARTIFACT_SUFFIX}"

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
        print_info "→ Rust build: $arch ($BUILD_TYPE, codegen=$CODEGEN_PROFILE)"
        "$RUST_BUILD_SCRIPT" "$arch" "$BUILD_TYPE" \
            --product-profile "$PRODUCT_PROFILE" \
            --codegen-profile "$CODEGEN_PROFILE" \
            "${WORKER_SNAPSHOT_ARGS[@]}"
    done

    print_success "Rust build done"
}

# ------------------------------------------------------------
# Validate JNI Libraries
# ------------------------------------------------------------
validate_native_libraries() {
    print_info "Validating $PRODUCT_PROFILE JNI libraries..."

    if [[ ! -d "$EXTERNAL_JNI_LIBS" ]]; then
        print_error "jniLibs directory not found: $EXTERNAL_JNI_LIBS"
        exit 1
    fi

    for arch in "${ARCHITECTURES[@]}"; do
        local src="$EXTERNAL_JNI_LIBS/$arch"

        if [[ ! -d "$src" ]]; then
            print_error "Missing native libs for $arch"
            exit 1
        fi
        for library in libmigo.so libc++_shared.so; do
            if [[ ! -f "$src/$library" ]]; then
                print_error "Missing $PRODUCT_PROFILE/$arch/$library"
                exit 1
            fi
        done
        print_success "$PRODUCT_PROFILE/$arch ready"
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
        if [[ ! -x "./gradlew" ]]; then
            chmod +x ./gradlew
        fi
        gradle_cmd="./gradlew"
    elif command -v gradle &> /dev/null; then
        gradle_cmd="gradle"
    else
        print_error "Gradle not found"
        exit 1
    fi

    $gradle_cmd clean

    local profile_task="${PRODUCT_PROFILE^}"
    local type_task="${BUILD_TYPE^}"
    local gradle_abis
    gradle_abis="$(IFS=,; echo "${ARCHITECTURES[*]}")"
    $gradle_cmd "-PmigoAbis=$gradle_abis" \
        "-PmigoCodegenProfile=$CODEGEN_PROFILE" \
        "-PmigoWorkerSnapshot=$WORKER_SNAPSHOT" \
        "assemble${profile_task}${type_task}"

    popd > /dev/null

    print_success "AAR build success"
}

# ------------------------------------------------------------
# Collect Outputs
# ------------------------------------------------------------
collect_outputs() {
    print_info "Collecting outputs..."

    local out_dir="$ANDROID_DIR/$OUTPUT_DIR"
    mkdir -p "$out_dir"

    local aar_dir="$LIBRARY_DIR/build/outputs/aar"
    local aar="$aar_dir/migo-$PRODUCT_PROFILE-$BUILD_TYPE.aar"
    if [[ ! -f "$aar" ]]; then
        print_error "Expected AAR not found: $aar"
        exit 1
    fi
    local artifact_name="migo-$PRODUCT_PROFILE-$BUILD_TYPE$ARTIFACT_SUFFIX.aar"
    cp "$aar" "$out_dir/$artifact_name"

    cat > "$out_dir/version-$PRODUCT_PROFILE$ARTIFACT_SUFFIX.json" << EOF
{
    "productProfile": "$PRODUCT_PROFILE",
    "buildType": "$BUILD_TYPE",
    "codegenProfile": "$CODEGEN_PROFILE",
    "cargoProfile": "$CARGO_PROFILE",
    "workerSnapshot": $WORKER_SNAPSHOT,
    "sourceDateEpoch": $SOURCE_DATE_EPOCH_JSON,
    "buildTime": "$(date '+%Y-%m-%d %H:%M:%S')"
}
EOF

    print_success "Outputs ready: $out_dir"
}

# ------------------------------------------------------------
# Main
# ------------------------------------------------------------
build_rust_library
validate_native_libraries
build_aar
collect_outputs

echo ""
echo "========================================"
print_success "Android AAR build completed"
echo "========================================"
