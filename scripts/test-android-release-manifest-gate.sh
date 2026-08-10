#!/usr/bin/env bash
# Android/Gradle contract: every release AAR producer must fail closed before
# expensive compilation unless scripts/build-aar.sh has staged verified inputs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$ROOT/platforms/android"

fail() {
  echo "Android release manifest gate: $*" >&2
  exit 1
}

# A release AAR must carry native libraries built from this source. `--skip-rust`
# packages whatever `.so` files are on disk and `validate_native_libraries` only
# checks that they exist, so a release built that way ships natives from another
# commit, profile or codegen setting with nothing in the artifact recording it. The
# refusal lives in `build-aar.sh` at argument time, which is what protects
# `release.yml` without a separate gate -- so what is checked here is that the
# refusal is still there, and that it is still specific enough to act on.
refuses() { # <expected substring> <build-aar.sh arguments...>
  local expected="$1"; shift
  local output status
  set +e
  output="$(bash "$ROOT/scripts/build-aar.sh" "$@" 2>&1)"
  status=$?
  set -e
  (( status != 0 )) || fail "build-aar.sh $* was accepted; expected: $expected"
  [[ "$output" == *"$expected"* ]] || \
    fail "build-aar.sh $* was refused for the wrong reason; expected: $expected"
}

refuses "Release AARs cannot be built with --skip-rust" --skip-rust release arm64-v8a
# The acknowledgement is only an acknowledgement when it acknowledges something.
refuses "only meaningful with --skip-rust" --unverified-native-libs release arm64-v8a
# `--unverified-native-libs` takes no value, so it must not consume the argument
# after it. It shifted inside its own case as well as at the loop tail, which
# silently dropped the next argument: an explicit architecture list vanished and the
# build widened to every ABI. The case above cannot see that, because what it
# swallows there (`release`) is a valid positional and also the default -- so the
# argument here is one that is not: with the extra shift `--artifact-manifest` is
# eaten and `off` is refused as an unknown positional instead.
refuses "Release AARs require --artifact-manifest required" \
    --unverified-native-libs --artifact-manifest off release

# The same contract through the other entry point. It is a separate script, so a
# refusal added to one is not a refusal added to both -- and this one is the path
# that had no manifest policy at all, which made every release it attempted fail
# inside Gradle rather than at argument time. Staging correctness needs no assertion
# here: the ps1 passes -PmigoVerifiedReleasePackaging, and Gradle's own
# verifyMigoReleaseArtifactPackaging task is what reads the staged inputs, so a
# script that staged the wrong thing fails there.
if command -v pwsh >/dev/null 2>&1; then
  ps_refuses() { # <expected substring> <build-aar.ps1 arguments...>
    local expected="$1"; shift
    local output status
    set +e
    output="$(pwsh -NoProfile -File "$ROOT/scripts/build-aar.ps1" "$@" 2>&1)"
    status=$?
    set -e
    (( status != 0 )) || fail "build-aar.ps1 $* was accepted; expected: $expected"
    [[ "$output" == *"$expected"* ]] || \
      fail "build-aar.ps1 $* was refused for the wrong reason; expected: $expected"
  }
  ps_refuses "Release AARs cannot be built with -SkipRustBuild" -BuildType release -SkipRustBuild
  ps_refuses "only meaningful with -SkipRustBuild" -UnverifiedNativeLibs
  ps_refuses "Release AARs require -ArtifactManifest required" \
      -BuildType release -ArtifactManifest optional
else
  echo "Android release manifest gate: SKIP pwsh absent, build-aar.ps1 not exercised"
fi

pushd "$ANDROID" >/dev/null

graph="$(./gradlew :library:bundleFullReleaseAar --dry-run)"
gate_line="$(awk '/:library:verifyMigoReleaseArtifactPackagingFull/{print NR; exit}' <<<"$graph")"
prebuild_line="$(awk '/:library:preFullReleaseBuild/{print NR; exit}' <<<"$graph")"
[[ -n "$gate_line" ]] || fail "bundleFullReleaseAar does not depend on the manifest gate"
[[ -n "$prebuild_line" ]] || fail "bundleFullReleaseAar task graph lacks preFullReleaseBuild"
(( gate_line < prebuild_line )) || fail "manifest gate must run before release compilation"

set +e
rejection="$(./gradlew :library:verifyMigoReleaseArtifactPackagingFull 2>&1)"
status=$?
set -e
(( status != 0 )) || fail "direct release packaging unexpectedly passed"
[[ "$rejection" == *"Direct release assembly is unsupported; use scripts/build-aar.sh"* ]] || \
  fail "direct release rejection was not actionable"

popd >/dev/null
echo "Android release manifest gate: ok"
