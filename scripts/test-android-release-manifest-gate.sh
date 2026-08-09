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
