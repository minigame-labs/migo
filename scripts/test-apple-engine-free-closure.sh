#!/usr/bin/env bash
# The engine-free layer must stay compilable for Apple targets from a machine
# that has no Apple SDK.
#
# `migo-frame-wire`, `migo-frame-decode` and `migo-capi-abi` are the three
# crates the iOS Performance+ product uses to receive, validate and decode a
# frame stream in a process that links no JavaScript engine. They are written
# and reviewed on Linux, and until this gate existed the only way to ask an
# Apple target whether they compile at all was to push and wait for the macOS
# runner -- or to own a Mac, which on the current plan is a purchase this
# project keeps deferring.
#
# THE DRIFT THIS EXISTS TO CATCH is not a bad line of Rust. It is a dependency:
#
#   migo-frame-decode -> migo-shared -> zstd -> zstd-sys
#
# `zstd-sys` is a C library. Its build script asks `cc` for a compiler for the
# target, and for `aarch64-apple-ios` `cc` shells out to `xcrun`, which does not
# exist on Linux. So the build died with `failed to find tool "xcrun"` -- an
# error that names the reader's toolchain and says nothing about the real cause,
# which is that a frame decoder had an archive format inside it. `migo-shared`
# is a large crate with many capabilities and `migo-frame-decode` wanted four
# types from it; it got a compressor, a hash suite and a signature verifier as
# well, all of them inside the trust boundary of a parser that reads bytes
# produced by content JavaScript in another process.
#
# `migo-frame-wire` next door states the rule this follows: every dependency a
# parser can reach is another thing inside its trust boundary. Its own closure
# is three packages, and its manifest says why.
#
# So this gate asserts the two halves of the fix:
#
#   1. The three crates cross-compile to `aarch64-apple-ios` HERE, on whatever
#      host is running, with no Apple SDK involved.
#   2. Their resolved dependency graphs contain none of the C libraries that
#      made (1) impossible, and have not quietly grown.
#
# Both halves fail closed. Half one is a real compile, so a missing target or a
# typo'd package name is a failure rather than a skip; half two is checked
# against a positive control, because a detector nobody has seen report a
# violation is a detector nobody should trust.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The layer, as Cargo package names.
ENGINE_FREE=(migo-frame-wire migo-frame-decode migo-capi-abi)

# The device target. Not the simulator: the simulator target happens to be
# reachable through more of rustc's own cross story, and the question is about
# the thing a phone runs.
APPLE_TARGET="aarch64-apple-ios"

# Packages that must not be reachable from the engine-free layer.
#
# Named individually rather than detected by "has a build script", because
# plenty of harmless crates have build scripts and a rule that flags them all is
# a rule someone will switch off. These are the ones that made the Apple build
# impossible, plus the hash suite that travels with them.
FORBIDDEN=(
    zstd
    zstd-sys
    ed25519-dalek
    sha1
    sha2
    md-5
    digest
    base64
)

# Ceilings, not targets. A closure is allowed to be smaller; growing past these
# means someone added a dependency to a parser, which is a decision that should
# be made on purpose and recorded here rather than noticed later.
#
# Measured 2026-09-05 on this tree, and these are the measurements: frame-wire
# 3, capi-abi 1, frame-decode 38 (57 before the filesystem and codec modules
# became optional). No headroom on purpose -- headroom is where a dependency
# lands without anyone deciding anything, and the cost of a false red here is
# one line of this file plus a sentence saying what the new package bought.
declare -A BUDGET=(
    [migo-frame-wire]=3
    [migo-capi-abi]=1
    [migo-frame-decode]=38
)

# The control. `migo-io` genuinely depends on the package format, so the
# detector below must report zstd there. If it does not, either FORBIDDEN lost
# an entry or the parsing broke, and every clean result above means nothing.
POSITIVE_CONTROL=migo-io
POSITIVE_CONTROL_EXPECTS=(zstd zstd-sys)

problems=()
notes=()

# Resolved package names for a closure, one per line.
#
# `-e normal`: a build- or dev-dependency on a C library does not put it in the
# product, and treating it as a violation makes the gate wrong in the direction
# that gets gates disabled.
resolve() {
    (cd engine && cargo tree -p "$1" -e normal --prefix none --color never --format '{p}' 2>&1)
}

package_names() {
    printf '%s\n' "$1" | awk 'NF {print $1}' | sort -u
}

# The single detector both the layer and the control go through. A control on a
# parallel path is a control of nothing.
violations_in() {
    local tree="$1" package banned
    while read -r package; do
        [[ -n "$package" ]] || continue
        for banned in "${FORBIDDEN[@]}"; do
            [[ "$package" == "$banned" ]] && echo "$package"
        done
    done < <(package_names "$tree")
}

# --- half one: it compiles for the device, here --------------------------------
#
# `check` and not `build`: producing a linked artifact for a physical iPhone
# needs a linker and a device, and neither is the question. The question is
# whether every crate in the graph can be compiled for that target by this host.
build_log="$(mktemp)"
trap 'rm -f "$build_log"' EXIT

cross_args=()
for package in "${ENGINE_FREE[@]}"; do cross_args+=(-p "$package"); done

if (cd engine && cargo check --locked --target "$APPLE_TARGET" ${cross_args[@]+"${cross_args[@]}"}) \
        >"$build_log" 2>&1; then
    notes+=("${ENGINE_FREE[*]} compile for $APPLE_TARGET on $(uname -s)")
else
    detail="$(tail -30 "$build_log" | sed 's/^/      /')"
    hint=""
    if grep -q "xcrun" "$build_log"; then
        hint="
      The error names xcrun, which means a C dependency came back into the
      closure. Half two below should say which one; if it does not, add it to
      FORBIDDEN -- the list is the part that decayed."
    elif grep -qE "target may not be installed|can't find crate for .core." "$build_log"; then
        hint="
      The target's standard library is missing. This is not a skip:
          rustup target add $APPLE_TARGET"
    fi
    problems+=("the engine-free layer does not compile for $APPLE_TARGET:
$detail$hint")
fi

# --- half two: the resolved graphs --------------------------------------------
for package in "${ENGINE_FREE[@]}"; do
    if ! tree="$(resolve "$package")"; then
        problems+=("cargo tree could not resolve $package:
$(printf '%s\n' "$tree" | sed 's/^/      /')")
        continue
    fi
    count="$(package_names "$tree" | wc -l)"
    budget="${BUDGET[$package]:-}"
    if [[ -z "$budget" ]]; then
        problems+=("$package has no closure budget; add one so growth is a decision")
    elif (( count > budget )); then
        problems+=("$package resolves $count packages, over its budget of $budget.
      Adding a dependency to a crate that parses content-produced bytes widens
      the parser's trust boundary. If the addition is intended, raise the number
      here and say what it bought.")
    else
        notes+=("$package resolves $count packages (budget $budget)")
    fi

    mapfile -t found < <(violations_in "$tree")
    for banned in ${found[@]+"${found[@]}"}; do
        [[ -n "$banned" ]] || continue
        why="$(cd engine && cargo tree -p "$package" -e normal --invert "$banned" 2>/dev/null | head -20 || true)"
        problems+=("$banned is reachable from $package:
$(printf '%s\n' "$why" | sed 's/^/      /')")
    done
done

# The control, through the same detector.
if ! control_tree="$(resolve "$POSITIVE_CONTROL")"; then
    problems+=("the positive control $POSITIVE_CONTROL does not resolve, so the detector is unverified:
$(printf '%s\n' "$control_tree" | sed 's/^/      /')")
else
    mapfile -t detected < <(violations_in "$control_tree")
    missing=()
    for expected in "${POSITIVE_CONTROL_EXPECTS[@]}"; do
        found_one=no
        for package in ${detected[@]+"${detected[@]}"}; do
            [[ "$package" == "$expected" ]] && { found_one=yes; break; }
        done
        [[ "$found_one" == yes ]] || missing+=("$expected")
    done
    if (( ${#missing[@]} > 0 )); then
        problems+=("the detector did not report ${missing[*]} in $POSITIVE_CONTROL, which does depend on them.
      Either FORBIDDEN lost an entry or the comparison broke; either way the
      clean results above prove nothing.")
    else
        notes+=("positive control: the detector reports ${POSITIVE_CONTROL_EXPECTS[*]} in $POSITIVE_CONTROL")
    fi
fi

# --- half three: the opt-out is still declared --------------------------------
#
# The whole arrangement rests on `migo-frame-decode` asking `migo-shared` for no
# default features. Restoring the default would put the archive format back
# without changing a line of Rust, and half one would then be the only thing
# that noticed -- on a host where it might not run.
# Asked of `cargo metadata`, not of the manifest text. A grep for the dependency
# line answers a question about formatting: reordering the keys, or splitting the
# entry across lines, changes the answer without changing the build. The resolver
# knows `uses_default_features` as a fact.
default_features_report="$(cd engine && cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | python3 -c '
import json, sys
meta = json.load(sys.stdin)
watched = {"migo-frame-decode", "migo-frame-wire", "migo-capi-abi"}
offenders = []
seen = []
for pkg in meta["packages"]:
    if pkg["name"] not in watched:
        continue
    for dep in pkg["dependencies"]:
        if dep["name"] != "migo-shared" or dep["kind"] is not None:
            continue
        seen.append(pkg["name"])
        if dep.get("uses_default_features", True):
            offenders.append(pkg["name"])
print("|".join(sorted(set(offenders))) + "#" + "|".join(sorted(set(seen))))
')"
offenders="${default_features_report%%#*}"
inspected="${default_features_report##*#}"
if [[ -z "$default_features_report" ]]; then
    problems+=("could not read uses_default_features out of cargo metadata, so the
      opt-out that keeps the archive format out of the decoder is unverified")
elif [[ -n "$offenders" ]]; then
    problems+=("${offenders//|/, } take migo-shared with its default features, which puts
      the virtual filesystem -- and the C compression library under it -- back
      inside a parser that reads content-produced bytes.")
elif [[ -z "$inspected" ]]; then
    problems+=("no crate in the engine-free layer depends on migo-shared at all, so this
      check inspected nothing. Either the layer was restructured or the package
      names here are stale.")
else
    notes+=("${inspected//|/, } take migo-shared without default features")
fi

printf '\n'
for note in ${notes[@]+"${notes[@]}"}; do echo "  - $note"; done
printf '\n'

if (( ${#problems[@]} > 0 )); then
    echo "FAIL: the engine-free layer is not Apple-buildable from this host." >&2
    printf '\n' >&2
    for problem in ${problems[@]+"${problems[@]}"}; do echo "  * $problem" >&2; done
    printf '\n' >&2
    cat >&2 <<'WHY'
  Why this matters: every Apple claim this repository can make today is made by
  compiling for an Apple target, because there is no Mac and no device. When the
  engine-free layer stops cross-compiling from Linux, the whole Apple side goes
  back to being checkable only on CI's macOS runner -- and, for anyone without
  one, not at all.
WHY
    exit 1
fi

echo "PASS: the engine-free layer compiles for $APPLE_TARGET and its parsers stay small."
