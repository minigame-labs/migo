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
