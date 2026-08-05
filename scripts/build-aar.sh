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
    echo "  ./build-aar.sh [--product-profile full|slim] [--codegen-profile z|2|3] [--worker-snapshot] [--artifact-manifest required|optional|off] [--build-type release|debug] [--output-dir dist] [--skip-rust] [architectures...]"
    echo ""
    echo "Options:"
    echo "  release|debug    Build type (default: release)"
    echo "  --build-type     Build type (release|debug)"
    echo "  --product-profile Product surface (full|slim, default: full)"
    echo "  --codegen-profile Hot-crate optimization (z|2|3, default: z; 2/3 require release)"
    echo "  --worker-snapshot Build the isolated full-release Worker snapshot candidate"
    echo "  --artifact-manifest Manifest policy (release requires required; debug defaults optional)"
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
MANIFEST_GENERATOR="$SCRIPT_DIR/generate-android-artifact-manifests.py"
BUILD_METADATA_WRITER="$SCRIPT_DIR/write-android-build-metadata.py"
AAR_MANIFEST_VERIFIER="$SCRIPT_DIR/verify-android-aar-manifests.py"
MANIFEST_TOOL_MANIFEST="$REPO_ROOT/tools/artifact-manifest/Cargo.toml"
MANIFEST_BUILD_ROOT="$LIBRARY_DIR/build/generated/migoArtifactManifest"
MANIFEST_ASSET_ROOT="$MANIFEST_BUILD_ROOT/assets/migo/artifacts"
MANIFEST_INDEX="$MANIFEST_ASSET_ROOT/package-index.json"
MANIFEST_TOOL=""
OUTPUT_DIR="dist"

# shellcheck source=scripts/lib/android-ndk.sh
source "$SCRIPT_DIR/lib/android-ndk.sh"

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
ARTIFACT_MANIFEST_MODE="${MIGO_ARTIFACT_MANIFEST_MODE:-}"
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
        --artifact-manifest)
            shift
            if [[ $# -eq 0 ]]; then
                print_error "--artifact-manifest requires a value"
                exit 1
            fi
            ARTIFACT_MANIFEST_MODE="$1"
            ;;
        --artifact-manifest=*)
            ARTIFACT_MANIFEST_MODE="${arg#*=}"
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
if [[ -z "$ARTIFACT_MANIFEST_MODE" ]]; then
    if [[ "$BUILD_TYPE" == "release" ]]; then
        ARTIFACT_MANIFEST_MODE="required"
    else
        ARTIFACT_MANIFEST_MODE="optional"
    fi
fi
if [[ "$ARTIFACT_MANIFEST_MODE" != "required" && "$ARTIFACT_MANIFEST_MODE" != "optional" && "$ARTIFACT_MANIFEST_MODE" != "off" ]]; then
    print_error "Invalid artifact manifest mode: $ARTIFACT_MANIFEST_MODE (expected required|optional|off)"
    exit 1
fi
if [[ "$BUILD_TYPE" == "release" && "$ARTIFACT_MANIFEST_MODE" != "required" ]]; then
    print_error "Release AARs require --artifact-manifest required"
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

# A single-ABI build gets its own artifact name so it cannot overwrite the
# multi-ABI one. Deliberately *not* folded into ARTIFACT_SUFFIX: that also names
# the jniLibs directory, and the arm64 `.so` is the same file whether or not
# x86_64 was built alongside it -- separating them there would rebuild Rust for
# nothing. Gradle's `abiFilters` does the actual filtering.
#
# Worth having because the number is a sales objection: 16-17 MB per ABI is the
# figure a host weighs against its own APK budget, and shipping only the
# multi-ABI AAR quotes it at double.
ABI_ARTIFACT_SUFFIX=""
if [[ ${#ARCHITECTURES[@]} -eq 1 ]]; then
    ABI_ARTIFACT_SUFFIX="-${ARCHITECTURES[0]}"
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
# Generate verified per-slice identities after Gradle clean
# ------------------------------------------------------------
resolve_source_revision() {
    local revision="${MIGO_SOURCE_REVISION:-${GITHUB_SHA:-}}"
    if [[ -z "$revision" ]]; then
        if ! revision="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null)"; then
            print_error "Cannot read the source revision; set MIGO_SOURCE_REVISION to the full commit" >&2
            return 1
        fi
    fi
    if [[ ! "$revision" =~ ^[0-9a-fA-F]{40}$ ]]; then
        print_error "A full MIGO_SOURCE_REVISION/GITHUB_SHA is required for artifact identity" >&2
        return 1
    fi
    printf '%s\n' "$revision"
}

generate_verified_artifact_manifests() {
    command -v cargo >/dev/null 2>&1 || { print_error "cargo is required for artifact manifests"; return 1; }
    command -v python3 >/dev/null 2>&1 || { print_error "python3 is required for artifact manifests"; return 1; }
    android_ndk_read_pin "$REPO_ROOT/contracts/artifact-manifest/android-v8.lock.json" \
        || { print_error "cannot read NDK pin"; return 1; }
    android_ndk_resolve || { print_error "cannot resolve the pinned Android NDK"; return 1; }
    [[ -f "$MANIFEST_GENERATOR" ]] || { print_error "Missing manifest generator: $MANIFEST_GENERATOR"; return 1; }
    [[ -f "$BUILD_METADATA_WRITER" ]] || { print_error "Missing build metadata writer: $BUILD_METADATA_WRITER"; return 1; }
    [[ -f "$AAR_MANIFEST_VERIFIER" ]] || { print_error "Missing AAR manifest verifier: $AAR_MANIFEST_VERIFIER"; return 1; }
    [[ -f "$MANIFEST_TOOL_MANIFEST" ]] || { print_error "Missing manifest tool: $MANIFEST_TOOL_MANIFEST"; return 1; }

    local tool_target="${MIGO_ARTIFACT_MANIFEST_TARGET_DIR:-$REPO_ROOT/tools/artifact-manifest/target}"
    CARGO_TARGET_DIR="$tool_target" cargo build \
        --manifest-path "$MANIFEST_TOOL_MANIFEST" --locked --release || return $?
    MANIFEST_TOOL="$tool_target/release/migo-artifact-manifest"
    [[ -x "$MANIFEST_TOOL" ]] || { print_error "Manifest tool was not produced: $MANIFEST_TOOL"; return 1; }

    local revision
    revision="$(resolve_source_revision)" || return $?
    python3 "$BUILD_METADATA_WRITER" \
        --repo-root "$REPO_ROOT" \
        --output "$MANIFEST_BUILD_ROOT/build-metadata.json" \
        --ndk-home "$ANDROID_NDK_HOME" \
        --source-revision "$revision" || return $?

    local -a generator_args=(
        --repo-root "$REPO_ROOT"
        --tool "$MANIFEST_TOOL"
        --output-root "$MANIFEST_ASSET_ROOT"
        --build-metadata "$MANIFEST_BUILD_ROOT/build-metadata.json"
        --product-profile "$PRODUCT_PROFILE"
        --build-type "$BUILD_TYPE"
        --codegen-profile "$CODEGEN_PROFILE"
    )
    if [[ "$WORKER_SNAPSHOT" == true ]]; then
        generator_args+=(--worker-snapshot)
    fi
    local arch
    for arch in "${ARCHITECTURES[@]}"; do
        generator_args+=(--arch "$arch")
    done
    python3 "$MANIFEST_GENERATOR" "${generator_args[@]}" || return $?
}

stage_artifact_manifests() {
    if [[ "$ARTIFACT_MANIFEST_MODE" == "off" ]]; then
        print_warning "Artifact manifest generation is disabled for this non-release build"
        return 0
    fi

    local manifest_rc=0
    generate_verified_artifact_manifests || manifest_rc=$?
    if [[ $manifest_rc -eq 0 ]]; then
        print_success "Verified artifact manifests staged"
        return 0
    fi
    rm -rf "$MANIFEST_BUILD_ROOT"
    if [[ "$ARTIFACT_MANIFEST_MODE" == "required" ]]; then
        print_error "Verified artifact manifest generation failed (rc=$manifest_rc)"
        return "$manifest_rc"
    fi
    print_warning "Verified artifact manifests unavailable; continuing debug-only build"
    return 0
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

    # clean removes generated assets; stage identity only after it succeeds.
    stage_artifact_manifests

    local profile_task="${PRODUCT_PROFILE^}"
    local type_task="${BUILD_TYPE^}"
    local gradle_abis
    local -a verified_release_args=()
    if [[ "$BUILD_TYPE" == "release" ]]; then
        verified_release_args=(
            -PmigoVerifiedReleasePackaging=true
            "-PmigoArtifactManifestTool=$MANIFEST_TOOL"
        )
    fi
    gradle_abis="$(IFS=,; echo "${ARCHITECTURES[*]}")"
    $gradle_cmd "-PmigoAbis=$gradle_abis" \
        "-PmigoCodegenProfile=$CODEGEN_PROFILE" \
        "-PmigoWorkerSnapshot=$WORKER_SNAPSHOT" \
        "${verified_release_args[@]}" \
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
    local artifact_name="migo-$PRODUCT_PROFILE-$BUILD_TYPE$ARTIFACT_SUFFIX$ABI_ARTIFACT_SUFFIX.aar"
    local output_aar="$out_dir/$artifact_name"
    local version_metadata="$out_dir/version-$PRODUCT_PROFILE$ARTIFACT_SUFFIX$ABI_ARTIFACT_SUFFIX.json"
    rm -f "$output_aar" "$output_aar.attestation.json" "$version_metadata"

    if [[ -f "$MANIFEST_INDEX" ]]; then
        python3 "$AAR_MANIFEST_VERIFIER" \
            --aar "$aar" \
            --index "$MANIFEST_INDEX" \
            --tool "$MANIFEST_TOOL"
    elif [[ "$ARTIFACT_MANIFEST_MODE" == "required" ]]; then
        print_error "Required package index was not generated: $MANIFEST_INDEX"
        exit 1
    fi

    cp "$aar" "$output_aar"
    if [[ -f "$MANIFEST_INDEX" ]]; then
        local attestation="$output_aar.attestation.json"
        "$MANIFEST_TOOL" attest "$output_aar" "$MANIFEST_INDEX" "$attestation" >/dev/null
        "$MANIFEST_TOOL" verify-attestation "$attestation" "$output_aar" "$MANIFEST_INDEX" >/dev/null
        print_success "Release attestation -> $attestation"
    fi

    cat > "$version_metadata" << EOF
{
    "productProfile": "$PRODUCT_PROFILE",
    "buildType": "$BUILD_TYPE",
    "codegenProfile": "$CODEGEN_PROFILE",
    "cargoProfile": "$CARGO_PROFILE",
    "workerSnapshot": $WORKER_SNAPSHOT,
    "artifactManifestMode": "$ARTIFACT_MANIFEST_MODE",
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
