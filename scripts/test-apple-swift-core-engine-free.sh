#!/usr/bin/env bash
# The engine-free Swift package must stay buildable without the engine.
#
# THE DRIFT THIS EXISTS TO CATCH already happened once, silently, for the whole
# life of the Apple skeleton. `platforms/apple/Package.swift` declares a
# `.binaryTarget` for MigoEngine.xcframework, which scripts/build-apple-sdk.sh
# produces and which needs macOS, Xcode and an hour of Skia. Until that artifact
# exists the package does not resolve, so NOTHING in it compiles -- and what was
# sitting in there uncompiled was `MigoDeploymentFloor.swift` and
# `MigoRuntimeProfile.swift`, the two files that mirror contracts/apple/*.json.
#
# Both have contract gates. Both gates compare TEXT. A Swift file can therefore
# agree with its contract to the character and still not build, which is the
# same shape as the ILP32 assertions in the C ABI lane: written twice, compiled
# once, and wrong in the arm nobody had compiled.
#
# So the sources that need no engine now live in their own package,
# `platforms/apple/core`, which apple-ci.yml builds and tests on the free macOS
# runner in seconds. This gate keeps that property true, because the way it will
# be lost is not a decision -- it is one `import MigoEngine`, or one new source
# directory that no target names, and either turns the lane back into something
# that compiles less than it appears to.
#
# Every check here carries a control that must fire, because a detector nobody
# has seen report a violation is a detector nobody should trust.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CORE="platforms/apple/core"
CORE_MANIFEST="$CORE/Package.swift"
SHIPPING_MANIFEST="platforms/apple/Package.swift"

# The module the xcframework vends, from the modulemap build-apple-sdk.sh
# writes, plus the C umbrella underneath it. Importing either is linking the
# engine.
ENGINE_MODULES=(MigoEngine migo)

# The two files whose contracts are text-only comparisons. They have to be in
# the package that actually compiles, or their gates go back to proving nothing.
MUST_COMPILE=(
    "$CORE/Sources/MigoAppleCore/MigoDeploymentFloor.swift"
    "$CORE/Sources/MigoAppleCore/MigoRuntimeProfile.swift"
)

problems=()
notes=()

[[ -f "$CORE_MANIFEST" ]] || { echo "FAIL: $CORE_MANIFEST does not exist" >&2; exit 1; }

# --- 1. the manifest takes nothing it cannot build ----------------------------
#
# One detector, run over both manifests. The shipping one is the control: it
# genuinely declares a binary target and a package dependency, so if the
# detector reports nothing there it is broken, and a clean core is meaningless.
declares_binary_target() { grep -c '\.binaryTarget(' "$1" || true; }
declares_package_dep()   { grep -c '\.package(path:\|\.package(url:' "$1" || true; }

core_binaries="$(declares_binary_target "$CORE_MANIFEST")"
core_packages="$(declares_package_dep "$CORE_MANIFEST")"
ship_binaries="$(declares_binary_target "$SHIPPING_MANIFEST")"
ship_packages="$(declares_package_dep "$SHIPPING_MANIFEST")"

if (( core_binaries != 0 )); then
    problems+=("$CORE_MANIFEST declares $core_binaries binary target(s). A binary target is a
      build output; declaring one here is what made the shipping package
      unbuildable until an hour of Skia had run.")
fi
if (( core_packages != 0 )); then
    problems+=("$CORE_MANIFEST declares $core_packages package dependencies. This package is the
      one that must resolve with nothing fetched and nothing built.")
fi
if (( ship_binaries == 0 || ship_packages == 0 )); then
    problems+=("the control failed: $SHIPPING_MANIFEST reports $ship_binaries binary target(s)
      and $ship_packages package dependencies, and it has both. The detector is not
      reading these manifests, so the clean result for the core package proves
      nothing.")
else
    notes+=("control: the detector sees $ship_binaries binary target(s) and $ship_packages package dependency in the shipping manifest")
    notes+=("$CORE_MANIFEST declares no binary target and no package dependency")
fi

# --- 2. nothing under the core package imports the engine ---------------------
#
# The control is `import XCTest`, which the test target does have. Without it, a
# scanner that matched nothing at all -- a wrong path, a typo'd pattern, a find
# that returned no files -- would read exactly like a clean result. This
# repository has produced two "decisive" false conclusions that way.
# `Package.swift` is excluded on purpose: it is a manifest SwiftPM executes, not
# a source SwiftPM compiles into a target, and counting it inflates the tally
# this gate reports as evidence that it inspected something.
mapfile -t core_sources < <(find "$CORE" -name '*.swift' -type f -not -name 'Package.swift' | sort)
if (( ${#core_sources[@]} == 0 )); then
    problems+=("no Swift sources found under $CORE; this gate inspected nothing")
else
    notes+=("scanned ${#core_sources[@]} Swift source(s) under $CORE")

    imports_of() { grep -hoE '^[[:space:]]*(@[A-Za-z_]+[[:space:]]+)?import[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' "$@" \
        | awk '{print $NF}' | sort -u; }
    mapfile -t found_imports < <(imports_of "${core_sources[@]}")

    control_hit=no
    for imported in "${found_imports[@]}"; do
        [[ "$imported" == "XCTest" ]] && control_hit=yes
    done
    if [[ "$control_hit" != yes ]]; then
        problems+=("the import scanner did not find 'import XCTest', which
      $CORE/Tests/MigoAppleCoreTests does contain. The scanner is not reading
      these files, so 'no engine import' below is not a finding.")
    else
        notes+=("control: the import scanner reports XCTest, which is present")
        for imported in "${found_imports[@]}"; do
            for engine in "${ENGINE_MODULES[@]}"; do
                if [[ "$imported" == "$engine" ]]; then
                    where="$(grep -lE "^[[:space:]]*(@[A-Za-z_]+[[:space:]]+)?import[[:space:]]+$engine\$" "${core_sources[@]}" | tr '\n' ' ')"
                    problems+=("$engine is imported under $CORE ($where).
      Code that calls the C ABI belongs in the shipping package, where the
      artifact it needs is declared; here it makes the lane stop building.")
                fi
            done
        done
    fi
fi

# --- 3. every source directory is a target ------------------------------------
#
# SwiftPM compiles what a target's path names and silently ignores the rest, so
# a new directory beside an existing one is a set of files that never reaches a
# compiler while the build stays green. Derived from both sides rather than
# listed here: a list would need the same maintenance as the thing it checks.
declared_targets="$(grep -oE 'path:[[:space:]]*"[^"]+"' "$CORE_MANIFEST" | sed 's/.*"\(.*\)"/\1/' | sort -u)"
for kind in Sources Tests; do
    [[ -d "$CORE/$kind" ]] || continue
    while read -r dir; do
        [[ -n "$dir" ]] || continue
        rel="$kind/$(basename "$dir")"
        if ! printf '%s\n' "$declared_targets" | grep -qx "$rel"; then
            problems+=("$CORE/$rel holds Swift sources that no target in $CORE_MANIFEST names,
      so SwiftPM never compiles them and the build stays green regardless.")
        fi
    done < <(find "$CORE/$kind" -mindepth 1 -maxdepth 1 -type d)
done
while read -r declared; do
    [[ -n "$declared" ]] || continue
    if [[ ! -d "$CORE/$declared" ]]; then
        problems+=("$CORE_MANIFEST names the target path '$declared', which does not exist")
    fi
done <<<"$declared_targets"
notes+=("target paths and source directories agree ($(printf '%s\n' "$declared_targets" | wc -l) declared)")

# --- 4. the text-only mirrors are in the package that compiles ----------------
for path in "${MUST_COMPILE[@]}"; do
    if [[ ! -f "$path" ]]; then
        problems+=("$path is not in the engine-free package. Its contract gate compares text,
      so outside a package that builds, nothing checks that it compiles at all.")
    fi
done
notes+=("the contract-mirrored Swift files live where they are compiled")

printf '\n'
for note in "${notes[@]}"; do echo "  - $note"; done
printf '\n'

if (( ${#problems[@]} > 0 )); then
    echo "FAIL: the engine-free Swift package is no longer engine-free." >&2
    printf '\n' >&2
    for problem in "${problems[@]}"; do echo "  * $problem" >&2; done
    printf '\n' >&2
    cat >&2 <<'WHY'
  Why this matters: this package is the only Swift in the repository that any
  compiler sees. The shipping package cannot resolve until an xcframework has
  been built on a Mac, so everything in it is unexamined by construction. Losing
  this lane means going back to a state where Swift agreed with its contracts on
  paper and had never been compiled.
WHY
    exit 1
fi

echo "PASS: the engine-free Swift package builds without the engine, and every source in it is compiled."
