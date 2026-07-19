#!/usr/bin/env bash
# Sourced, not executed. Pins every host compilation at the Debian bullseye
# sysroot so the produced artifacts stay inside the glibc 2.31 loader floor
# (docs/multiplatform-architecture.md 7.2).
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

MIGO_SYSROOT="${MIGO_SYSROOT:-/home/xg/wkspace/rusty_v8_src/build/linux/debian_bullseye_amd64-sysroot}"

migo_sysroot_require() {
    if [[ ! -d "$MIGO_SYSROOT" ]]; then
        echo "[linux-sdk] sysroot not found: $MIGO_SYSROOT" >&2
        echo "[linux-sdk] It ships with the Chromium/V8 checkout under" >&2
        echo "[linux-sdk]   <rusty_v8_src>/build/linux/debian_bullseye_amd64-sysroot" >&2
        echo "[linux-sdk] Set MIGO_SYSROOT to a Debian bullseye (glibc 2.31) sysroot." >&2
        return 1
    fi
    local libstdcxx="$MIGO_SYSROOT/usr/lib/x86_64-linux-gnu/libstdc++.so.6"
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
# falls through to the host GCC's copy under /usr/lib/gcc/x86_64-linux-gnu/13,
# which lives outside the sysroot and therefore needs GLIBC_2.34 symbols the
# sysroot's libc does not define -- the link then fails with a wall of
# "disallowed by --no-allow-shlib-undefined". The sysroot's own
# libstdc++.so.6.0.28 is exactly the GLIBCXX_3.4.28 floor.
migo_sysroot_link_dir() {
    local link_dir="$1"
    local lib_dir="$MIGO_SYSROOT/usr/lib/x86_64-linux-gnu"
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
            "$MIGO_SYSROOT/lib/x86_64-linux-gnu/lib${stem}.so."* \
            2>/dev/null | sort -V | tail -1 || true)"
        if [[ -n "$versioned" ]]; then
            ln -sf "$versioned" "$link_dir/lib${stem}.so"
        fi
    done
    return 0
}

migo_sysroot_export() {
    migo_sysroot_require || return 1

    export MIGO_SYSROOT
    export CC="${CC_HOST:-/usr/bin/clang}"
    export CXX="${CXX_HOST:-/usr/bin/clang++}"

    local sysroot_flag="--sysroot=$MIGO_SYSROOT"
    export CFLAGS="${CFLAGS:-} $sysroot_flag"
    export CXXFLAGS="${CXXFLAGS:-} $sysroot_flag"
    # The cc crate reads the target-suffixed forms in preference to the plain
    # ones, so both are set or the suffixed empty value wins.
    export CFLAGS_x86_64_unknown_linux_gnu="$CFLAGS"
    export CXXFLAGS_x86_64_unknown_linux_gnu="$CXXFLAGS"

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
    # path pointing at the host's /usr/lib/x86_64-linux-gnu, which precedes the
    # sysroot on the link line -- so `-lc` resolves to the host's glibc 2.39
    # while the crt startup object comes from the sysroot's 2.31. The link then
    # fails on __libc_csu_init / __libc_csu_fini, which glibc removed in 2.34.
    #
    # PKG_CONFIG_LIBDIR replaces the default search path rather than extending
    # it, so no host .pc file can be picked up; PKG_CONFIG_SYSROOT_DIR rewrites
    # the -L and -I answers to point inside the sysroot.
    export PKG_CONFIG_SYSROOT_DIR="$MIGO_SYSROOT"
    export PKG_CONFIG_LIBDIR="$MIGO_SYSROOT/usr/lib/pkgconfig"
    PKG_CONFIG_LIBDIR+=":$MIGO_SYSROOT/usr/lib/x86_64-linux-gnu/pkgconfig"
    PKG_CONFIG_LIBDIR+=":$MIGO_SYSROOT/usr/share/pkgconfig"

    # rustc must both compile and link against the sysroot. -B points the driver
    # at the sysroot's crt objects; rpath-link resolves transitive DT_NEEDED at
    # link time without baking a build-machine path into the artifact.
    local lib_dir="$MIGO_SYSROOT/usr/lib/x86_64-linux-gnu"
    RUSTFLAGS_SYSROOT_LINK="-C link-arg=$sysroot_flag"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-B$lib_dir"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-L$lib_dir"
    RUSTFLAGS_SYSROOT_LINK+=" -C link-arg=-Wl,-rpath-link,$lib_dir"
    export RUSTFLAGS_SYSROOT_LINK
}
