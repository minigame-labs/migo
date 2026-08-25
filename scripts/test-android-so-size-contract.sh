#!/usr/bin/env bash
# The size of the shipped libmigo.so is nobody's assertion, and it has cost us
# three times.
#
#   * `slim_icu_data()` swaps Skia's 8.5 MB `android` ICU blob for the 761 KB
#     `flutter` one. Its `find -maxdepth` was one level short of where
#     skia-bindings actually unpacks, so for a while it silently did nothing and
#     the .so carried 7.7 MB it was supposed to have dropped. Nothing was red.
#   * `engine/.cargo/config.toml` declares 13 rustflags for each Android target.
#     Setting `RUSTFLAGS` *replaces* that list rather than adding to it, and
#     `build-android-so.sh` -- the one build that ships -- set it from scratch.
#     `--gc-sections` and `--icf=all` had therefore never run on a released
#     binary: 3.47 MB, 8% of the image. Nothing was red. (#118)
#   * The AAR shipped ~1 MB per ABI of `libc++_shared.so` that no shipped .so
#     names in DT_NEEDED. That one now has its own gate. (#116)
#
# Each was found by someone going looking, each got a mechanism-specific fix,
# and not one of them would have been caught a second time. The common failure
# is not any of those mechanisms -- it is that a build can grow by megabytes and
# every test still passes.
#
# So this gate asserts the artifact, not the mechanism. Ceilings are per ABI, on
# the whole file and on the three sections where all three regressions landed,
# because a total alone says only "it grew" while a section says which machinery
# stopped working:
#
#   .text      code size -- `--gc-sections` / `--icf=all` / the codegen profile
#   .rodata    embedded data -- the ICU blob, the V8 startup snapshot
#   .rela.dyn  the relocation table -- whether relative relocations are packed
#
# Per-section ceilings also catch a regression that a total would hide, where
# one mechanism breaks while another saves a comparable number of bytes.
#
# The relocation ceiling is deliberately expressed as "the flat table is small",
# not "APS2 is in use". Today `--pack-dyn-relocs=android` packs 90,430 relative
# relocations from 2,173,992 bytes down to 321,967. If minSdk ever reaches 30,
# SHT_RELR encodes the same offsets in ~18 KB and would be strictly better --
# an improvement must not turn this gate red. Note also that a stock GNU readelf
# prints the Android tags as `Operating System specific: 60000011`, so matching
# on a tag name would be a check that quietly passes on the wrong host.
#
# The numbers below are headroom over a measured build, not aspirations. Raising
# one is a decision: put the new measurement and the reason next to it. Growth
# that is real should move these; growth that nobody can explain is the thing
# this gate exists to stop.
#
# Release artifacts only. A debug .so is ~340 MB and has no budget worth
# writing, so debug AARs are skipped by name -- and because a gate that skips
# everything and reports success is the failure mode this repository keeps
# rediscovering, checking zero ABI payloads is an error, not a pass.
#
# Usage: scripts/test-android-so-size-contract.sh [aar ...]
#   With no arguments, checks every release AAR in platforms/android/dist/.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

fail() { echo "FAIL: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Budgets. Measured on 2026-08-24 against the v0.9.4 tree with relative
# relocations packed, then rounded up for headroom:
#
#   arm64-v8a  file 40,204,256  .text 29,237,096  .rodata 7,838,779  .rela.dyn 321,967
#   x86_64     file 43,042,784  .text 31,722,125  .rodata 8,093,379  .rela.dyn 327,543
#
# ~5% on the file and on .text/.rodata: ordinary feature work moves .text by
# tens of kilobytes, so this absorbs many releases, while every regression in
# the header above (3.47 MB, 7.7 MB, 1.85 MB) blows straight through it.
#
# The relocation budget is a flat 600,000 bytes rather than a percentage. It has
# only two states in practice -- packed, or a flat table an order of magnitude
# larger -- so a tight bound costs nothing and names the cause exactly.
# ---------------------------------------------------------------------------
declare -A BUDGET_FILE=(
    ["arm64-v8a"]=42200000
    ["x86_64"]=45200000
)
declare -A BUDGET_TEXT=(
    ["arm64-v8a"]=30700000
    ["x86_64"]=33300000
)
declare -A BUDGET_RODATA=(
    ["arm64-v8a"]=8250000
    ["x86_64"]=8500000
)
RELOC_BUDGET=600000

# Which mechanism to look at first when a given section is over.
budget_hint() {
    case "$1" in
        file)
            echo "The sections below narrow it. If they are all inside their budgets,
      something new is being shipped inside the binary."
            ;;
        .text)
            echo "Code size. Check that config.toml's rustflags reached the linker
      (scripts/test-android-rustflags-reach-the-linker.sh proves the wiring, not
      the result), that --gc-sections and --icf=all are in the link, and that
      the codegen profile is still 'z'."
            ;;
        .rodata)
            echo "Embedded data. The usual causes are the ICU blob (slim_icu_data in
      build-android-so.sh must leave the 761 KB flutter icudtl.dat in place of
      Skia's 8.5 MB android one) and the V8 startup snapshot."
            ;;
        .rela.dyn|.rel.dyn)
            echo "The relative relocations are being shipped as a flat table. Check
      that --pack-dyn-relocs=android survived into the link -- setting RUSTFLAGS
      anywhere in the build replaces config.toml's list wholesale, which is how
      thirteen other flags were lost once already."
            ;;
    esac
}

resolve_readelf() {
    local ndk="${ANDROID_NDK_HOME:-}"
    if [[ -n "$ndk" ]]; then
        local candidate
        candidate="$(find "$ndk/toolchains/llvm/prebuilt" -name llvm-readelf -type f 2>/dev/null | head -1)"
        [[ -n "$candidate" ]] && { echo "$candidate"; return 0; }
    fi
    local found
    found="$(find "$HOME/Android" -name llvm-readelf -type f 2>/dev/null | head -1)"
    [[ -n "$found" ]] && { echo "$found"; return 0; }
    command -v llvm-readelf 2>/dev/null && return 0
    command -v readelf 2>/dev/null && return 0
    return 1
}

READELF="$(resolve_readelf)" || fail "no readelf found (set ANDROID_NDK_HOME, or install binutils)"

section_size() {
    # Sum every section whose name matches exactly, by name only: the type
    # column is what changes when relocations get packed, and naming a type
    # here would make this a check on the encoding rather than on the bytes.
    "$READELF" --section-headers --wide "$1" | python3 -c '
import re, sys
want = sys.argv[1]
total = 0
for line in sys.stdin:
    m = re.match(r"\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+[0-9a-f]+\s+[0-9a-f]+\s+([0-9a-f]+)", line)
    if m and m.group(1) == want:
        total += int(m.group(2), 16)
print(total)
' "$2"
}

# --json also emits the measurement this gate already takes, so the numbers a
# customer-facing page shows come from the same readelf pass that gates them
# rather than from someone retyping them. Enforcement is NOT skipped in this
# mode: a measurement that could be produced by an over-budget build would be a
# way to publish exactly the number nobody wanted to see.
JSON_MODE=false
args=()
for arg in "$@"; do
    case "$arg" in
        --json) JSON_MODE=true ;;
        *) args+=("$arg") ;;
    esac
done
set -- ${args[@]+"${args[@]}"}
# In JSON mode the progress lines would corrupt the document, so they go to stderr.
say() { if $JSON_MODE; then echo "$@" >&2; else echo "$@"; fi; }

aars=("$@")
if [[ ${#aars[@]} -eq 0 ]]; then
    while IFS= read -r f; do aars+=("$f"); done < <(
        find "$REPO_ROOT/platforms/android/dist" -maxdepth 1 -name '*.aar' 2>/dev/null | sort
    )
fi
[[ ${#aars[@]} -gt 0 ]] || fail "no AAR to check.
      Build one first: bash scripts/build-aar.sh --product-profile full release"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

checked=0
json_rows=()
for aar in "${aars[@]}"; do
    [[ -f "$aar" ]] || fail "not a file: $aar"
    name="$(basename "$aar")"

    if [[ "$name" == *-debug* ]]; then
        say "  $name: debug artifact; no size budget"
        continue
    fi

    mapfile -t sos < <(unzip -Z1 "$aar" 'jni/*/libmigo.so' 2>/dev/null | sort || true)
    if [[ ${#sos[@]} -eq 0 ]]; then
        # The derived -nojni AAR carries no engine; its own gate covers what it
        # must not contain.
        say "  $name: no engine payload (derived artifact); skipped"
        continue
    fi

    rm -rf "${work:?}"/*
    unzip -qo "$aar" 'jni/*/libmigo.so' -d "$work"

    for entry in "${sos[@]}"; do
        abi="$(cut -d/ -f2 <<<"$entry")"
        so="$work/$entry"

        [[ -n "${BUDGET_FILE[$abi]:-}" ]] || fail "$name ships ABI '$abi', which has no size budget.
      Add one to this script from a measured build rather than leaving the ABI
      unchecked -- an ABI with no ceiling is an ABI nobody is watching."

        file_size="$(stat -c %s "$so")"
        text_size="$(section_size "$so" .text)"
        rodata_size="$(section_size "$so" .rodata)"
        # Whichever encoding the linker chose, the flat table is what must stay
        # small. .rel.dyn is the 32-bit spelling.
        reloc_size=$(( $(section_size "$so" .rela.dyn) + $(section_size "$so" .rel.dyn) ))

        over=()
        (( file_size   <= ${BUDGET_FILE[$abi]}   )) || over+=("file|$file_size|${BUDGET_FILE[$abi]}")
        (( text_size   <= ${BUDGET_TEXT[$abi]}   )) || over+=(".text|$text_size|${BUDGET_TEXT[$abi]}")
        (( rodata_size <= ${BUDGET_RODATA[$abi]} )) || over+=(".rodata|$rodata_size|${BUDGET_RODATA[$abi]}")
        (( reloc_size  <= RELOC_BUDGET           )) || over+=(".rela.dyn|$reloc_size|$RELOC_BUDGET")

        if (( ${#over[@]} > 0 )); then
            {
                echo "FAIL: $name [$abi] libmigo.so is over budget."
                for item in "${over[@]}"; do
                    IFS='|' read -r what actual limit <<<"$item"
                    printf '\n  %-9s %'"'"'d bytes, budget %'"'"'d (over by %'"'"'d)\n' \
                        "$what" "$actual" "$limit" "$((actual - limit))"
                    echo "      $(budget_hint "$what")"
                done
                echo "
      If this growth is intended, raise the budget in
      scripts/test-android-so-size-contract.sh and record the measurement and
      the reason beside it. Do not raise it to make a red build green without
      knowing which of the numbers above moved and why."
            } >&2
            exit 1
        fi

        say "$(printf '  %s [%s]: %d bytes (.text %d, .rodata %d, reloc %d)' \
            "$name" "$abi" "$file_size" "$text_size" "$rodata_size" "$reloc_size")"

        if $JSON_MODE; then
            # The rest of the .so, for anyone drawing the whole thing rather than
            # gating three numbers of it.
            data_size=$(( $(section_size "$so" .data) + $(section_size "$so" .data.rel.ro) ))
            bss_size="$(section_size "$so" .bss)"
            unwind_size=$(( $(section_size "$so" .eh_frame) + $(section_size "$so" .eh_frame_hdr) ))
            jar_size="$(unzip -l "$aar" classes.jar 2>/dev/null | awk '/classes\.jar/{print $1; exit}')"
            aar_uncompressed="$(unzip -l "$aar" 2>/dev/null | tail -1 | awk '{print $1}')"
            json_rows+=("$(printf '{"aar":"%s","abi":"%s","file":%d,"text":%d,"rodata":%d,"reloc":%d,"data":%d,"bss":%d,"unwind":%d,"classes_jar":%d,"aar_uncompressed":%d,"aar_file":%d}' \
                "$name" "$abi" "$file_size" "$text_size" "$rodata_size" "$reloc_size" \
                "$data_size" "$bss_size" "$unwind_size" "${jar_size:-0}" "${aar_uncompressed:-0}" "$(stat -c %s "$aar")")")
        fi
        checked=$((checked + 1))
    done
done

if $JSON_MODE; then
    printf '{\n  "measured": "%s",\n  "source": "scripts/test-android-so-size-contract.sh --json",\n  "note": "raw byte counts; every payload below passed the size gate",\n  "artifacts": [\n' \
        "$(date -u +%Y-%m-%d)"
    for i in "${!json_rows[@]}"; do
        printf '    %s%s\n' "${json_rows[$i]}" "$([[ $i -lt $(( ${#json_rows[@]} - 1 )) ]] && echo ,)"
    done
    printf '  ]\n}\n'
fi

(( checked > 0 )) || fail "checked no ABI payload.
      Every AAR given was skipped, and a gate that skips everything reports
      success without having looked at anything. Point this at a release AAR:
      bash scripts/build-aar.sh --product-profile full release"

say "PASS: every shipped libmigo.so is inside its size budget ($checked ABI payloads checked)"
