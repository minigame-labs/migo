#!/usr/bin/env bash
# The setup docs must not name a concrete Rust/rustc version that disagrees with
# the toolchain pin.
#
# `engine/rust-toolchain.toml` pins `channel` to an exact rustc version and
# rustup honours it automatically -- so the pin is a build contract, not a
# checklist. But CONTRIBUTING.md / BUILD.md carried a hand-written "Rust 1.80+
# (edition 2024)" that no gate tied to the pin. It was wrong twice over: the pin
# had moved to 1.95.0, and edition 2024 needs rustc >= 1.85, not 1.80 -- a
# stranger following the prose could install a toolchain that cannot build the
# workspace. BUILD.md also called the pin "stable".
#
# The rule: on a line mentioning Rust/rustc/rustup in a tracked setup doc, the
# only `1.NN` tokens allowed are the pinned channel's major.minor and `1.85`
# (the fixed edition-2024 floor, worth stating). Anything else is drift -- name
# the toolchain file instead of a number.
#
# Scope: CONTRIBUTING.md, CONTRIBUTING.zh-CN.md, BUILD.md.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

toolchain_file="engine/rust-toolchain.toml"
channel="$(grep -oE '^channel *= *"[0-9]+\.[0-9]+(\.[0-9]+)?"' "$toolchain_file" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?')"
if [[ -z "$channel" ]]; then
    echo "ERROR: could not read the channel pin from $toolchain_file." >&2
    exit 1
fi
channel_mm="${channel%.*}"          # 1.95.0 -> 1.95
edition_floor="1.85"                # edition 2024 stabilised in rustc 1.85

docs=(CONTRIBUTING.md CONTRIBUTING.zh-CN.md BUILD.md)
offenders=()

for doc in "${docs[@]}"; do
    [[ -f "$doc" ]] || continue
    while IFS=: read -r lineno line; do
        # every 1.NN token on a Rust-mentioning line
        while read -r ver; do
            [[ -n "$ver" ]] || continue
            if [[ "$ver" != "$channel_mm" && "$ver" != "$edition_floor" ]]; then
                offenders+=("$doc:$lineno: names Rust $ver (pin is $channel; edition floor is $edition_floor)")
            fi
        done < <(grep -oE '1\.[0-9]{2}' <<<"$line" || true)
    done < <(grep -nE '[Rr]ust|rustc|rustup' "$doc" || true)
done

if [[ ${#offenders[@]} -gt 0 ]]; then
    echo "ERROR: a setup doc names a Rust version that disagrees with $toolchain_file." >&2
    echo "Point at the toolchain file instead of a number:" >&2
    printf '  %s\n' "${offenders[@]}" >&2
    exit 1
fi

echo "Doc Rust-version contract: PASS (setup docs agree with the $channel pin / $edition_floor edition floor)"
