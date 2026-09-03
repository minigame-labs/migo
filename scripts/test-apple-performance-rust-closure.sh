#!/usr/bin/env bash
# The Performance+ product must contain no embedded JavaScript engine.
#
# This is the one claim the whole iOS fast lane rests on. Content JavaScript runs
# inside WebKit's WebContent process to get the system's JIT; the renderer runs
# here, in the app process, and it is allowed to be here *because* it is not a
# second JavaScript engine. An iOS build that quietly linked V8 would be
# shipping two engines, paying twice for working set and package size, and
# claiming a memory advantage it had just spent.
#
# THE DRIFT THIS EXISTS TO CATCH is not somebody adding V8 on purpose. It is a
# convenience dependency three levels down: `migo-capi -> migo-core ->
# migo-runtime-v8 -> deno_core` was the shape when this gate was written, and
# nothing about `migo-core`'s public surface says so. The dependency is a fact
# about the resolved graph, so only the resolved graph can answer it.
#
# It measures the SHIPPING PRODUCT, not a proxy. An earlier plan for this gate
# had it check a small probe crate that depended on the external-frame feature.
# A probe's closure being clean says nothing about the product's closure -- the
# question is what `migo-capi` resolves to when built the way the iOS product is
# built, and that is what is asked here.
#
# Host-only for the Cargo half. The archive half needs macOS tooling and runs
# when --artifact points at a built static library; it does not silently pass
# when it cannot run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Package identities that must not appear. Matched against the package NAME
# from `cargo tree --prefix none --format {p}`, not against a line of tree art:
# a substring search over the rendered tree matches the comment characters and
# the version string too, and `v8` as a substring matches nothing useful at all.
FORBIDDEN=(
    migo-runtime-v8
    migo-runtime-jsc
    deno_core
    deno_ops
    v8
    rusty_v8
    v8_valueserializer
)

# The closures that must be free of an engine, as `package:features`.
#
# `migo-capi` is the one that matters: it is what the Apple SDK links, so it is
# what the product claim is about. `migo-core` is kept alongside it because a
# violation there is the more likely one and the message names the smaller
# graph, which is easier to read than the same package appearing three levels
# down in the larger one.
CLEAN_CLOSURES=(
    "migo-capi:external-frames"
    "migo-core:external-frames"
)

# And the control. `migo-core` built the ordinary way *does* embed V8, so this
# gate must be able to see it there.
#
# Without this, every way of breaking the detection -- a renamed package, a
# cargo-tree format change, an awk field that moved, a typo in the forbidden
# list -- reads as a pass. This repository has been bitten by exactly that
# shape often enough to have a note about it: a guard nobody has seen fail is a
# guard nobody should trust, and the cheapest way to see it fail is to keep a
# case in the gate that must fail.
POSITIVE_CONTROL="migo-capi:profile-full"
POSITIVE_CONTROL_EXPECTS=(migo-runtime-v8 deno_core v8)

artifact=""
while (( $# > 0 )); do
    case "$1" in
        --artifact) artifact="${2:?--artifact needs a path}"; shift 2 ;;
        --artifact=*) artifact="${1#*=}"; shift ;;
        *) echo "usage: $0 [--artifact <path to a built static library>]" >&2; exit 2 ;;
    esac
done

problems=()
notes=()

# --- half one: the resolved dependency graph ---------------------------------
#
# `-e normal` on purpose: a build-dependency or a dev-dependency on V8 does not
# put V8 in the shipped archive, and treating it as a violation would make the
# gate wrong in the direction that gets gates disabled.
resolve() {
    local package="${1%%:*}" features="${1#*:}"
    (cd engine && cargo tree \
        -p "$package" \
        --no-default-features \
        --features "$features" \
        -e normal \
        --prefix none \
        --format '{p}' 2>&1)
}

# The detector, and there is exactly one of it.
#
# Both the clean closures and the positive control go through this function, so
# the control exercises the same comparison the verdict rests on. An earlier
# version checked the control against the resolved tree directly, and deleting
# an entry from FORBIDDEN then went unnoticed: the control confirmed V8 was in
# the embedded build and never asked whether this gate's list would have caught
# it. A control on a parallel path is a control of nothing.
#
# Prints one banned package name per line, or nothing.
violations_in() {
    local tree="$1" package banned
    while read -r package; do
        [[ -n "$package" ]] || continue
        for banned in "${FORBIDDEN[@]}"; do
            [[ "$package" == "$banned" ]] && echo "$package"
        done
    done < <(printf '%s\n' "$tree" | awk 'NF {print $1}' | sort -u)
}

package_count() {
    printf '%s\n' "$1" | awk 'NF {print $1}' | sort -u | wc -l
}

for target in "${CLEAN_CLOSURES[@]}"; do
    if ! tree="$(resolve "$target")"; then
        problems+=("cargo tree could not resolve $target:
$(printf '%s\n' "$tree" | sed 's/^/      /')")
        continue
    fi
    count="$(package_count "$tree")"
    notes+=("$target resolves $count packages")
    if (( count < 20 )); then
        problems+=("$target resolved only $count packages; an almost-empty closure is not a clean one")
    fi
    mapfile -t found < <(violations_in "$tree")
    for banned in "${found[@]}"; do
        [[ -n "$banned" ]] || continue
        why="$(cd engine && cargo tree \
            -p "${target%%:*}" --no-default-features \
            --features "${target#*:}" -e normal \
            --invert "$banned" 2>/dev/null | head -20 || true)"
        problems+=("$banned is in the $target closure:
$(printf '%s\n' "$why" | sed 's/^/      /')")
    done
done

# The control: the embedded product must still be reported as containing the
# engine, BY THE DETECTOR ABOVE. If it is not, either the detector broke or the
# embedded product stopped embedding, and both are things to learn from a
# failure here rather than from a clean run that means nothing.
if ! control_tree="$(resolve "$POSITIVE_CONTROL")"; then
    problems+=("the positive control $POSITIVE_CONTROL does not resolve, so the detector is unverified:
$(printf '%s\n' "$control_tree" | sed 's/^/      /')")
else
    mapfile -t detected < <(violations_in "$control_tree")
    missing=()
    for expected in "${POSITIVE_CONTROL_EXPECTS[@]}"; do
        found_one=no
        for package in "${detected[@]}"; do
            [[ "$package" == "$expected" ]] && { found_one=yes; break; }
        done
        [[ "$found_one" == yes ]] || missing+=("$expected")
    done
    if (( ${#missing[@]} > 0 )); then
        problems+=("the detector did not report ${missing[*]} in $POSITIVE_CONTROL, which does contain them.
      Either FORBIDDEN lost an entry or the comparison broke; either way a clean
      result above proves nothing.")
    else
        notes+=("positive control: the detector reports ${POSITIVE_CONTROL_EXPECTS[*]} in $POSITIVE_CONTROL")
    fi
fi

# --- half two: the built archive ---------------------------------------------
#
# The graph is necessary and not sufficient: a vendored object, a static
# library committed as a binary, or a linker script could put engine symbols in
# an archive whose Cargo graph is clean. Only the artifact answers that, and
# only on a host with the tooling.
if [[ -n "$artifact" ]]; then
    if [[ ! -f "$artifact" ]]; then
        problems+=("--artifact $artifact does not exist")
    else
        nm_tool=""
        for candidate in llvm-nm llvm-nm-18 nm; do
            if command -v "$candidate" >/dev/null 2>&1; then nm_tool="$candidate"; break; fi
        done
        if [[ -z "$nm_tool" ]]; then
            problems+=("--artifact was given but no nm is available; this is not a pass")
        else
            symbols="$("$nm_tool" --defined-only "$artifact" 2>/dev/null || true)"
            if [[ -z "$symbols" ]]; then
                problems+=("$nm_tool listed no defined symbols in $artifact; this is not a pass")
            else
                # Engine entry points, not the string "v8": a Rust symbol path
                # containing v8 appears in our own module names.
                for pattern in '_ZN2v8' 'v8::internal::' 'deno_core::' 'JSGlobalContextCreate' 'JSEvaluateScript'; do
                    if printf '%s\n' "$symbols" | grep -qF "$pattern"; then
                        problems+=("$artifact defines $pattern, so it embeds a JavaScript engine")
                    fi
                done
                notes+=("archive audited with $nm_tool: $(printf '%s\n' "$symbols" | wc -l) defined symbols")
            fi
        fi
    fi
else
    notes+=("archive audit skipped: no --artifact given (the Cargo half still ran)")
fi

printf '\n'
for note in "${notes[@]}"; do echo "  - $note"; done
printf '\n'

if (( ${#problems[@]} > 0 )); then
    echo "FAIL: the Performance+ product is not free of an embedded JavaScript engine." >&2
    printf '\n' >&2
    for problem in "${problems[@]}"; do echo "  * $problem" >&2; done
    printf '\n' >&2
    cat >&2 <<'WHY'
  Why this matters: MigoApplePerformancePlus exists because content JavaScript
  runs in WebKit's WebContent process, where the system grants a JIT this
  process cannot have. A build that also links an engine here pays for two,
  gets a JIT for neither, and cannot make the memory claim the lane is for.
WHY
    exit 1
fi

echo "PASS: no JavaScript engine in the Performance+ closure."
