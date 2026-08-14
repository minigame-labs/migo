#!/usr/bin/env bash
# Sourced, not executed. Pins every host compilation at the Debian bullseye
# sysroot so the produced artifacts stay inside the glibc 2.31 loader floor
# that scripts/abi-floor-audit.py enforces.
#
# This is the sysroot Chromium ships and the one the linux-gnu V8 archive was
# built in, so the engine's C++ and V8's C++ see the same libc and libstdc++
# headers. That consistency is why this sysroot is used rather than a container
# image built from an arbitrary Debian 11 base.
#
# Measured 2026-07-19: without it, migo's artifacts require up to GLIBC_2.38 and
# GLIBCXX_3.4.31, from ICU/freetype/harfbuzz compiled against the host's
# stdlib.h, Skia compiled against GCC 13's libstdc++ headers, and pthread/dl
# symbols resolved against a post-2.34 libc.
#
# Arch is threaded through explicitly (MIGO_SYSROOT_KEY / MIGO_SYSROOT_TARGET),
# not detected from the host: the reason for pinning an old sysroot at all is
# to keep the *shipped artifact's* floor low regardless of what glibc the
# build machine happens to carry, which is exactly as true building natively
# on an arm64 CI runner as it is cross-compiling arm64 from this x86_64 one --
# `--target=` is passed to clang explicitly either way, never left to match
# whatever the host defaults to.

_MIGO_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Both the recipe and the tree it describes live in the repository, materialised by
# scripts/fetch-linux-sysroot.sh. They used to be resolved inside a sibling
# ../rusty_v8_src checkout, which is what made a Linux package producible only on a
# machine that happened to have one -- and the reason the packaging gate is
# documented as release-machine-only.
MIGO_SYSROOT_HOME="${MIGO_SYSROOT_HOME:-$_MIGO_REPO_ROOT/engine/third_party/linux-sysroot}"
MIGO_SYSROOT_KEY="${MIGO_SYSROOT_KEY:-bullseye_amd64}"
MIGO_SYSROOT_RECIPE="${MIGO_SYSROOT_RECIPE:-$MIGO_SYSROOT_HOME/sysroots.json}"
# MIGO_SYSROOT is deliberately not defaulted here. Which directory the tarball
# unpacks into is the recipe's answer, and repeating it would be a second pin that
# can disagree with the one whose sha256 is the recorded sysroot identity.

# One key, three vocabularies that disagree: sysroots.json's key
# (bullseye_amd64/bullseye_arm64), the Debian multiarch lib directory name
# (x86_64-linux-gnu/aarch64-linux-gnu) and the rust target triple
# (x86_64-unknown-linux-gnu/aarch64-unknown-linux-gnu) that also has to name a
# clang --target and a cc-crate CFLAGS_<target> env var suffix. Same set of
# keys fetch-linux-sysroot.sh's SYSROOT_LIB_TRIPLETS uses, kept in sync by
# both being small and both being the only two Linux SDK targets this repo
# builds -- widen one, widen the other.
declare -A _MIGO_SYSROOT_LIB_TRIPLETS=(
    [bullseye_amd64]="x86_64-linux-gnu"
    [bullseye_arm64]="aarch64-linux-gnu"
)
declare -A _MIGO_SYSROOT_RUST_TARGETS=(
    [bullseye_amd64]="x86_64-unknown-linux-gnu"
    [bullseye_arm64]="aarch64-unknown-linux-gnu"
)
declare -A _MIGO_SYSROOT_WORDS=(
    [bullseye_amd64]="amd64"
    [bullseye_arm64]="arm64"
)

migo_sysroot_require() {
    if [[ -z "${_MIGO_SYSROOT_LIB_TRIPLETS[$MIGO_SYSROOT_KEY]:-}" ]]; then
        echo "[linux-sdk] unsupported sysroot key: $MIGO_SYSROOT_KEY" >&2
        echo "[linux-sdk] known: ${!_MIGO_SYSROOT_LIB_TRIPLETS[*]}" >&2
        return 1
    fi
    MIGO_SYSROOT_LIB_TRIPLET="${_MIGO_SYSROOT_LIB_TRIPLETS[$MIGO_SYSROOT_KEY]}"
    MIGO_SYSROOT_RUST_TARGET="${_MIGO_SYSROOT_RUST_TARGETS[$MIGO_SYSROOT_KEY]}"

    if [[ ! -f "$MIGO_SYSROOT_RECIPE" ]]; then
        echo "[linux-sdk] Chromium sysroot recipe not found: $MIGO_SYSROOT_RECIPE" >&2
        return 1
    fi
    local sysroot_dir sysroot_url
    # One read for both answers the recipe holds about this key: where the tarball
    # unpacks, and the URL an installed tree records in its .stamp.
    if ! IFS=$'\t' read -r sysroot_dir sysroot_url < <(
        python3 - "$MIGO_SYSROOT_RECIPE" "$MIGO_SYSROOT_KEY" <<'PY'
import json
import sys

recipe, key = sys.argv[1], sys.argv[2]
with open(recipe, encoding="utf-8") as handle:
    entry = json.load(handle).get(key)
if not entry:
    sys.exit(f"no sysroot named {key} in {recipe}")
missing = [field for field in ("SysrootDir", "URL", "Sha256Sum") if not entry.get(field)]
if missing:
    sys.exit(f"{key} is missing {', '.join(missing)}")
print(entry["SysrootDir"], entry["URL"] + "/" + entry["Sha256Sum"], sep="\t")
PY
    ); then
        return 1
    fi
    : "${MIGO_SYSROOT:=$MIGO_SYSROOT_HOME/$sysroot_dir}"
    if [[ ! -d "$MIGO_SYSROOT" ]]; then
        echo "[linux-sdk] sysroot not found: $MIGO_SYSROOT" >&2
        echo "[linux-sdk] Materialise it with: bash scripts/fetch-linux-sysroot.sh --key $MIGO_SYSROOT_KEY" >&2
        return 1
    fi
    # The identity this recipe hashes into is written into the package manifest and
    # is required to equal the one the V8 component recorded, so it is a claim about
    # which sysroot the artifact was compiled against. Checking only that some
    # libstdc++ exists would let MIGO_SYSROOT point at an unrelated Debian tree and
    # still have the package swear it was the pinned Chromium one. The .stamp both
    # this repo's fetch script and Chromium's own install-sysroot.py write is what
    # ties a tree back to the pin, so an override is accepted only when it carries
    # the one this recipe names.
    local stamp="$MIGO_SYSROOT/.stamp"
    if [[ ! -f "$stamp" ]]; then
        echo "[linux-sdk] sysroot carries no .stamp, so it cannot be tied to the pin: $MIGO_SYSROOT" >&2
        echo "[linux-sdk] Materialise it with: bash scripts/fetch-linux-sysroot.sh --key $MIGO_SYSROOT_KEY" >&2
        return 1
    fi
    if [[ "$(cat "$stamp")" != "$sysroot_url" ]]; then
        echo "[linux-sdk] sysroot was installed from a different pin than $MIGO_SYSROOT_RECIPE names" >&2
        echo "[linux-sdk] recipe:    $sysroot_url" >&2
        echo "[linux-sdk] installed: $(cat "$stamp")" >&2
        return 1
    fi
    local libstdcxx="$MIGO_SYSROOT/usr/lib/$MIGO_SYSROOT_LIB_TRIPLET/libstdc++.so.6"
    if [[ ! -f "$libstdcxx" ]]; then
        echo "[linux-sdk] sysroot is missing libstdc++: $libstdcxx" >&2
        return 1
    fi
}

# The sysroot ships runtime sonames (libEGL.so.1, libstdc++.so.6.0.28) but not
# every unversioned development symlink a `-l<name>` needs. Rather than mutate a
# shared checkout, the links are staged in a directory of our own.
#
# libstdc++ is the load-bearing one. Without a libstdc++.so here, `-lstdc++`
# falls through to the host GCC's copy under /usr/lib/gcc/<triplet>/13, which
# lives outside the sysroot and therefore needs newer glibc symbols the
# sysroot's libc does not define -- the link then fails with a wall of
# "disallowed by --no-allow-shlib-undefined". The sysroot's own
# libstdc++.so.6.0.28 is exactly the GLIBCXX_3.4.28 floor.
migo_sysroot_link_dir() {
    local link_dir="$1"
    local lib_dir="$MIGO_SYSROOT/usr/lib/$MIGO_SYSROOT_LIB_TRIPLET"
    mkdir -p "$link_dir"
    # Not every stem exists in the sysroot (libgcc_s ships only as a runtime
    # under a different path, for instance). A missing one is not an error --
    # the driver falls back to its own copy, and the floor audit is what decides
    # whether that fallback is acceptable. The `if` is deliberate: a trailing
    # `[[ -n ... ]] && ln` would make `set -e` abort the whole build on the first
    # stem that happens to be absent.
    # The sysroot splits runtime libraries across /lib and /usr/lib; libgcc_s
    # lives only under /lib, so both are searched.
    local stem versioned
    for stem in stdc++ gcc_s EGL GL GLESv2 X11 asound fontconfig freetype; do
        if [[ -e "$link_dir/lib${stem}.so" ]]; then
            continue
        fi
        versioned="$(ls -1 \
            "$lib_dir/lib${stem}.so."* \
            "$MIGO_SYSROOT/lib/$MIGO_SYSROOT_LIB_TRIPLET/lib${stem}.so."* \
            2>/dev/null | sort -V | tail -1 || true)"
        if [[ -n "$versioned" ]]; then
            ln -sf "$versioned" "$link_dir/lib${stem}.so"
        fi
    done
    return 0
}

# Resolves to a path, on stdout, or fails with a diagnostic on stderr. Shared
# by every Linux link step -- both this file's own RUSTFLAGS_SYSROOT_LINK and
# build-linux-sdk.sh's direct libmigo.so link -- so there is exactly one
# answer to "which lld" rather than two independent searches that could
# silently disagree (say, MIGO_LLD honoured by one call site and not the
# other). GNU ld is never an acceptable fallback here, for two unrelated
# reasons measured on two unrelated links: it emits a malformed
# symbol-version table for the libmigo.so link (build-linux-sdk.sh), and the
# system's own binutils ld on this machine was built without AArch64
# emulation support at all -- `unrecognised emulation mode: aarch64linux` --
# which breaks the cross-compiled rustc link even for a plain executable.
migo_sysroot_lld_bin() {
    local lld_bin="${MIGO_LLD:-$(command -v ld.lld 2>/dev/null || true)}"
    if [[ -z "$lld_bin" ]]; then
        local bundled="$_MIGO_REPO_ROOT/../rusty_v8_src/target/$MIGO_SYSROOT_RUST_TARGET/release/clang/bin/ld.lld"
        [[ -x "$bundled" ]] && lld_bin="$bundled"
    fi
    if [[ -z "$lld_bin" ]]; then
        echo "[linux-sdk] no lld found. Install it (apt-get install lld) or set MIGO_LLD." >&2
        echo "[linux-sdk] A system lld is preferred; the rusty_v8 toolchain copy is only a" >&2
        echo "[linux-sdk] convenience for machines that do not have one." >&2
        return 1
    fi
    printf '%s\n' "$lld_bin"
}

migo_sysroot_export() {
    migo_sysroot_require || return 1

    local sysroot_word="${_MIGO_SYSROOT_WORDS[$MIGO_SYSROOT_KEY]}"
    # Record a relocatable recipe identity, never a builder-specific absolute
    # path. The Linux package verifier requires the engine and V8 component to
    # name this exact same identity.
    MIGO_SYSROOT_IDENTITY="Debian bullseye $sysroot_word sysroot; sysroots.json sha256=$(sha256sum "$MIGO_SYSROOT_RECIPE" | awk '{print $1}')"
    export MIGO_SYSROOT
    export MIGO_SYSROOT_IDENTITY
    export CC="${CC_HOST:-/usr/bin/clang}"
    export CXX="${CXX_HOST:-/usr/bin/clang++}"

    # clang is a native cross-compiler; --target is passed explicitly rather
    # than left to match the host so this produces the same artifact whether
    # run cross-compiling on x86_64 or natively on an arm64 runner.
    local clang_target_flag="--target=$MIGO_SYSROOT_LIB_TRIPLET"
    local sysroot_flag="--sysroot=$MIGO_SYSROOT"
    export CFLAGS="${CFLAGS:-} $clang_target_flag $sysroot_flag"
    export CXXFLAGS="${CXXFLAGS:-} $clang_target_flag $sysroot_flag"
    # The cc crate reads the target-suffixed forms in preference to the plain
    # ones, so both are set or the suffixed empty value wins. The suffix is the
    # rust target triple with every `-` replaced by `_`, per the cc crate's own
    # convention -- computed, not hand-spelled, so a third target added here
    # later cannot typo it.
    local target_suffix="${MIGO_SYSROOT_RUST_TARGET//-/_}"
    printf -v "CFLAGS_${target_suffix}" '%s' "$CFLAGS"
    printf -v "CXXFLAGS_${target_suffix}" '%s' "$CXXFLAGS"
    export "CFLAGS_${target_suffix}" "CXXFLAGS_${target_suffix}"

    # skia-bindings drives its own gn/ninja build and has first-class sysroot
    # support: SDKTARGETSYSROOT is appended to the same cflag list it builds, and
    # is also used to rewrite freetype's hard-coded /usr/include/freetype2 include
    # path to the sysroot copy (skia-bindings build_support/skia/config.rs).
    #
    # Do NOT try to inject the sysroot through SKIA_GN_ARGS instead: that sets
    # extra_cflags as a whole list, skia-bindings already populates it, and gn
    # rejects the result with "Replacing nonempty list".
    export SDKTARGETSYSROOT="$MIGO_SYSROOT"

    # Crates that locate system libraries through pkg-config (alsa-sys) must be
    # answered from the sysroot too. Without this, alsa-sys emits a link search
    # path pointing at the host's own multiarch lib directory, which precedes
    # the sysroot on the link line -- so `-lc` resolves to the host's newer
    # glibc while the crt startup object comes from the sysroot's 2.31. The
    # link then fails on __libc_csu_init / __libc_csu_fini, which glibc removed
    # in 2.34.
    #
    # PKG_CONFIG_LIBDIR replaces the default search path rather than extending
    # it, so no host .pc file can be picked up; PKG_CONFIG_SYSROOT_DIR rewrites
    # the -L and -I answers to point inside the sysroot.
    export PKG_CONFIG_SYSROOT_DIR="$MIGO_SYSROOT"
    export PKG_CONFIG_LIBDIR="$MIGO_SYSROOT/usr/lib/pkgconfig"
    PKG_CONFIG_LIBDIR+=":$MIGO_SYSROOT/usr/lib/$MIGO_SYSROOT_LIB_TRIPLET/pkgconfig"
    PKG_CONFIG_LIBDIR+=":$MIGO_SYSROOT/usr/share/pkgconfig"

    # rustc must both compile and link against the sysroot. -B points the driver
    # at the sysroot's crt objects; rpath-link resolves transitive DT_NEEDED at
    # link time without baking a build-machine path into the artifact.
    #
    # `-C linker=$CC` and `--target=` are both required for cross-compilation
    # specifically (arm64 on this x86_64 host), not merely for consistency:
    # rustc's default linker driver for a Linux target is bare `cc` with no
    # `--target=` of its own, which happens to produce correct output for a
    # native x86_64 build purely because the host is also x86_64 -- and
    # cross-compiling to arm64 exposed that this was never actually pinned.
    # Measured: without these two, `migo-c-host-example`'s link (the only
    # binary this script produces -- migo-capi itself is a staticlib with no
    # final link to expose the gap) failed with `rust-lld: error:
    # crtbeginS.o is incompatible with elf64-littleaarch64`, `cc` having
    # picked crt objects from the host's own /usr/lib/gcc/x86_64-linux-gnu/13
    # ahead of anything -B or -L named -- -B affects search order among
    # candidates a target-aware driver already considers, not which target a
    # target-unaware one compiles for.
    # The system's default `ld` this rustc link would otherwise fall through
    # to is not a safe assumption on every machine: measured here, it is a
    # binutils build carrying only x86 emulations, so a cross-compiled arm64
    # link fails with `unrecognised emulation mode: aarch64linux` even though
    # clang was given the correct --target and --sysroot. lld supports every
    # target uniformly, which is also why build-linux-sdk.sh's own libmigo.so
    # link already insists on it (see migo_sysroot_lld_bin).
    local lld_bin
    lld_bin="$(migo_sysroot_lld_bin)" || return 1

    local lib_dir="$MIGO_SYSROOT/usr/lib/$MIGO_SYSROOT_LIB_TRIPLET"
    RUSTFLAGS_SYSROOT_LINK="-C linker=$CC"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=$clang_target_flag"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=--ld-path=$lld_bin"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=$sysroot_flag"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-B$lib_dir"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-L$lib_dir"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-Wl,-rpath-link,$lib_dir"
    export RUSTFLAGS_SYSROOT_LINK
}
