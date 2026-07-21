#!/usr/bin/env bash
# Host-only Q14 selective-codegen and artifact-isolation contract gate.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE="$ROOT/engine"
ANDROID_SO="$ROOT/scripts/build-android-so.sh"
AAR="$ROOT/scripts/build-aar.sh"
ANDROID_SO_PS="$ROOT/scripts/build-android-so.ps1"
AAR_PS="$ROOT/scripts/build-aar.ps1"
GRADLE="$ROOT/platforms/android/library/build.gradle"

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

require_literal() {
    local file="$1"
    local literal="$2"
    local description="$3"
    grep -Fq -- "$literal" "$file" || fail "$description"
}

expect_rejection() {
    local expected="$1"
    shift
    local output
    if output=$("$@" 2>&1); then
        fail "command unexpectedly succeeded: $*"
    fi
    [[ "$output" == *"$expected"* ]] \
        || fail "command did not report '$expected': $*"
}

echo "[1/5] checking Cargo selective release profiles"
python3 - "$ENGINE/Cargo.toml" <<'PY'
import pathlib
import sys
import tomllib

path = pathlib.Path(sys.argv[1])
with path.open("rb") as stream:
    document = tomllib.load(stream)

profiles = document.get("profile", {})
release = profiles.get("release")
if not isinstance(release, dict) or release.get("opt-level") != "z":
    raise SystemExit("release baseline must remain opt-level=z")

hot_packages = {"migo-audio", "migo-core", "migo-graphics", "migo-io", "migo-runtime-v8"}
for profile_name, expected_level in (("release-hot2", 2), ("release-hot3", 3)):
    profile = profiles.get(profile_name)
    if not isinstance(profile, dict):
        raise SystemExit(f"missing [profile.{profile_name}]")
    if profile.get("inherits") != "release":
        raise SystemExit(f"{profile_name} must inherit release")

    build_override = profile.get("build-override", {})
    if build_override.get("opt-level") != "z" or build_override.get("codegen-units") != 1:
        raise SystemExit(f"{profile_name} build override must remain z/1")

    packages = profile.get("package", {})
    wildcard = packages.get("*", {})
    if wildcard.get("opt-level") != "z" or wildcard.get("codegen-units") != 1:
        raise SystemExit(f"{profile_name} dependency wildcard must remain z/1")

    named = {name for name in packages if name != "*"}
    if named != hot_packages:
        raise SystemExit(
            f"{profile_name} hot package set mismatch: {sorted(named)}"
        )
    for package in hot_packages:
        override = packages[package]
        if set(override) != {"opt-level"} or override["opt-level"] != expected_level:
            raise SystemExit(
                f"{profile_name}.{package} must only set opt-level={expected_level}"
            )
PY

echo "[2/5] checking native build entrypoints"
for script in "$ANDROID_SO" "$ANDROID_SO_PS"; do
    require_literal "$script" "--codegen-profile" "$(basename "$script") lacks --codegen-profile"
    require_literal "$script" "release-hot2" "$(basename "$script") lacks hot2 mapping"
    require_literal "$script" "release-hot3" "$(basename "$script") lacks hot3 mapping"
    require_literal "$script" "opt2" "$(basename "$script") lacks opt2 output isolation"
    require_literal "$script" "opt3" "$(basename "$script") lacks opt3 output isolation"
done
require_literal "$ANDROID_SO" 'codegen_profile="z"' "Bash native builder must default to z"
require_literal "$ANDROID_SO_PS" '$codegenProfile = "z"' "PowerShell native builder must default to z"
require_literal "$ANDROID_SO_PS" '$buildType = "release"' \
    "PowerShell native builder must match the Bash release default"
require_literal "$ANDROID_SO_PS" 'throw "Unknown argument: $arg"' \
    "PowerShell native builder must reject unknown arguments"
require_literal "$ANDROID_SO_PS" '$locationPushed = $false' \
    "PowerShell native builder must track cargo working-directory cleanup"
require_literal "$ANDROID_SO_PS" 'Print-Error "Unable to start cargo build for $platform`: $_"' \
    "PowerShell native builder must handle Start-Process exceptions"
require_literal "$ANDROID_SO" '--target-dir "$TARGET_DIR"' \
    "Bash native builder must pin Cargo output to the copied target root"
require_literal "$ANDROID_SO_PS" '"--target-dir", $paths.Target' \
    "PowerShell native builder must pin Cargo output to the copied target root"
require_literal "$ANDROID_SO" \
    'local src_so="$TARGET_DIR/$target_triple/$out_dir/$CRATE_SO_NAME"' \
    "Bash native copy source must use the selected Cargo profile directory"
require_literal "$ANDROID_SO_PS" \
    '$srcSo = Join-Path $paths.Target "$targetTriple\$outDir\$CRATE_SO_NAME"' \
    "PowerShell native copy source must use the selected Cargo profile directory"

python3 - "$ANDROID_SO" "$ANDROID_SO_PS" <<'PY'
import pathlib
import sys

bash = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
powershell = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")

def required_index(text, needle, description, start=0):
    position = text.find(needle, start)
    if position < 0:
        raise SystemExit(description)
    return position


bash_so_missing = required_index(
    bash,
    'print_error "Output .so not found: $src_so"',
    "Bash native builder must error on a missing exact .so",
)
bash_so_return = required_index(
    bash,
    "return 1",
    "Bash native builder must return failure for a missing .so",
    bash_so_missing,
)
bash_libcpp_missing = required_index(
    bash,
    'print_error "libc++_shared.so not found: $libcpp_src"',
    "Bash native builder must error on a missing libc++_shared.so",
    bash_so_return,
)
bash_libcpp_return = required_index(
    bash,
    "return 1",
    "Bash native builder must return failure for a missing libc++_shared.so",
    bash_libcpp_missing,
)
if not bash_so_missing < bash_so_return < bash_libcpp_missing < bash_libcpp_return:
    raise SystemExit("Bash native builder must fail closed on missing exact outputs")

ps_so_missing = required_index(
    powershell,
    'Print-Error "Output .so not found: $srcSo"',
    "PowerShell native builder must error on a missing exact .so",
)
ps_so_return = required_index(
    powershell,
    "return $false",
    "PowerShell native builder must return failure for a missing .so",
    ps_so_missing,
)
ps_libcpp_missing = required_index(
    powershell,
    'Print-Error "libc++_shared.so not found: $libcppSrc"',
    "PowerShell native builder must error on a missing libc++_shared.so",
    ps_so_return,
)
ps_libcpp_return = required_index(
    powershell,
    "return $false",
    "PowerShell native builder must return failure for a missing libc++_shared.so",
    ps_libcpp_missing,
)
if not ps_so_missing < ps_so_return < ps_libcpp_missing < ps_libcpp_return:
    raise SystemExit("PowerShell native builder must fail closed on missing exact outputs")

ps_dependency_call = required_index(
    powershell, "\nCheck-Dependencies\n", "PowerShell dependency call is missing"
)
ps_main = required_index(powershell, "# Main", "PowerShell main section is missing")
ps_argument_loop = required_index(
    powershell,
    "for ($i = 0; $i -lt $Args.Count; $i++)",
    "PowerShell argument loop is missing",
    ps_main,
)
ps_default_release = required_index(
    powershell,
    '$buildType = "release"',
    "PowerShell native builder must default to release",
    ps_main,
)
if not ps_main < ps_default_release < ps_argument_loop:
    raise SystemExit("PowerShell native builder must default to release before parsing")

ps_codegen_validation = required_index(
    powershell,
    'if ($codegenProfile -notin @("z", "2", "3"))',
    "PowerShell codegen validation is missing",
)
ps_debug_validation = required_index(
    powershell,
    'if ($buildType -eq "debug" -and $codegenProfile -ne "z")',
    "PowerShell debug/codegen validation is missing",
)
if not ps_codegen_validation < ps_debug_validation < ps_dependency_call:
    raise SystemExit(
        "PowerShell native builder must reject invalid/debug codegen before dependencies"
    )
PY

echo "[3/5] checking AAR and Gradle routing"
for script in "$AAR" "$AAR_PS"; do
    require_literal "$script" "codegenProfile" "$(basename "$script") metadata lacks codegen identity"
    require_literal "$script" "cargoProfile" "$(basename "$script") metadata lacks Cargo identity"
    require_literal "$script" "migoCodegenProfile" "$(basename "$script") does not forward the Gradle property"
    require_literal "$script" "opt2" "$(basename "$script") lacks opt2 artifact isolation"
    require_literal "$script" "opt3" "$(basename "$script") lacks opt3 artifact isolation"
done
require_literal "$AAR" 'CODEGEN_PROFILE="z"' "Bash AAR builder must default to z"
require_literal "$AAR_PS" '$CodegenProfile = "z"' "PowerShell AAR builder must default to z"
require_literal "$GRADLE" "migoCodegenProfile" "Gradle lacks the Q14 selection property"
require_literal "$GRADLE" "supportedMigoCodegenProfiles" "Gradle does not validate Q14 values"
require_literal "$GRADLE" "migoNativeProfileSuffix" "Gradle does not isolate native roots"
require_literal "$GRADLE" 'jniLibs.srcDirs = []' "Gradle main JNI source must stay empty"
require_literal "$GRADLE" "SOURCE_DATE_EPOCH" \
    "Gradle BuildInfo generation must support reproducible A/B timestamps"
require_literal "$GRADLE" 'inputs.property("sourceDateEpoch", sourceDateEpoch)' \
    "Gradle BuildInfo task must track SOURCE_DATE_EPOCH"
require_literal "$AAR" '"sourceDateEpoch": $SOURCE_DATE_EPOCH_JSON' \
    "Bash AAR metadata must record SOURCE_DATE_EPOCH"
require_literal "$AAR_PS" 'sourceDateEpoch = $SourceDateEpochMetadata' \
    "PowerShell AAR metadata must record SOURCE_DATE_EPOCH"
for script in "$AAR" "$AAR_PS"; do
    require_literal "$script" "Invalid SOURCE_DATE_EPOCH" \
        "$(basename "$script") must reject invalid reproducible timestamps"
done

echo "[4/5] checking fail-closed CLI behavior"
expect_rejection "expected z|2|3" bash "$ANDROID_SO" --codegen-profile invalid
expect_rejection "expected z|2|3" bash "$AAR" --codegen-profile invalid
expect_rejection "requires a release build" bash "$ANDROID_SO" debug --codegen-profile 2
expect_rejection "requires a release build" bash "$AAR" debug --codegen-profile 2
expect_rejection "Invalid SOURCE_DATE_EPOCH" env SOURCE_DATE_EPOCH=not-a-time \
    bash "$AAR" --skip-rust arm64-v8a

echo "[5/5] checking shell syntax"
bash -n "$ANDROID_SO" "$AAR" "$ROOT/scripts/test-product-profiles.sh" "$0"

echo "PASS: Q14 selective codegen profiles and artifacts are isolated"
