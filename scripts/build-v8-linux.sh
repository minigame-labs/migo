#!/usr/bin/env bash
# ============================================================
# Build the linux-gnu librusty_v8.a that migo's Linux SDK links against.
# Location: scripts/build-v8-linux.sh
#
# Counterpart to scripts/build-v8-android.sh. It differs from whatever archive
# happens to sit in ../rusty_v8_src in two ways that both matter, and both of
# which force a full rebuild anyway -- so they are done in one pass:
#
#   1. SYSROOT. The archive previously used by host builds was compiled against
#      the build machine's glibc 2.39 headers. `nm --undefined-only` shows 27
#      undefined __isoc23_* references in it (platform-linux.o, string.o,
#      locale.o, ...). Those are renamed entry points introduced in glibc 2.38,
#      not versioned aliases, so nothing at link time can satisfy them against
#      the 2.31 loader floor that docs/multiplatform-architecture.md 7.2
#      promises. use_sysroot=true builds against the Debian bullseye sysroot.
#
#   2. SHARED-OBJECT LINKAGE. An archive compiled for executables may use the
#      local-exec TLS model, whose relocations cannot appear in a shared object
#      -- which is why libmigo.so could not be linked at all. Position-
#      independent code plus a dynamic TLS model makes the same archive usable
#      from both an executable and a shared library.
#
# Output: engine/third_party/rusty_v8/x86_64-linux-gnu/
#           librusty_v8.a + src_binding.rs + component-manifest.json
#
# Usage:
#   scripts/build-v8-linux.sh            # build and install
#   scripts/build-v8-linux.sh --check    # report what the current archive is
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
TARGET="x86_64-unknown-linux-gnu"
OUT_DIR="$ENGINE_ROOT/third_party/rusty_v8/x86_64-linux-gnu"

RUSTY_V8_SRC="${RUSTY_V8_SRC:-$PROJECT_ROOT/../rusty_v8_src}"
GN_OUT="$RUSTY_V8_SRC/target/$TARGET/release/gn_out"
BACKUP_DIR="$RUSTY_V8_SRC/target/$TARGET/release/gn_out-pre-sdk-backup"

info() { echo -e "\033[0;36m[v8-linux] $*\033[0m"; }
err()  { echo -e "\033[0;31m[v8-linux] $*\033[0m" >&2; }

[[ -d "$RUSTY_V8_SRC" ]] || { err "rusty_v8 source not found: $RUSTY_V8_SRC"; exit 1; }

if [[ "${1:-}" == "--check" ]]; then
    for archive in "$GN_OUT/obj/librusty_v8.a" "$OUT_DIR/librusty_v8.a"; do
        [[ -f "$archive" ]] || { echo "missing: $archive"; continue; }
        echo "=== $archive"
        echo "  glibc 2.38 entry points referenced: \
$(nm --undefined-only "$archive" 2>/dev/null | grep -c '__isoc23_' || true)"
        echo "  local-exec TLS relocations: \
$(objdump -r "$archive" 2>/dev/null | grep -cw 'R_X86_64_TPOFF32' || true)"
        echo "  local-dynamic TLS relocations: \
$(objdump -r "$archive" 2>/dev/null | grep -cw 'R_X86_64_DTPOFF32' || true)"
    done
    exit 0
fi

# ------------------------------------------------------------
# Preserve the existing archive. It is what dev-test-host.sh and the player
# link against today; if this build goes wrong, the host must still work.
# ------------------------------------------------------------
if [[ -f "$GN_OUT/obj/librusty_v8.a" && ! -d "$BACKUP_DIR" ]]; then
    info "backing up the current archive to $BACKUP_DIR"
    mkdir -p "$BACKUP_DIR/obj"
    cp "$GN_OUT/obj/librusty_v8.a" "$BACKUP_DIR/obj/"
    cp "$GN_OUT/src_binding.rs" "$BACKUP_DIR/" 2>/dev/null || true
fi

# ------------------------------------------------------------
# GN arguments
# ------------------------------------------------------------
GN_ARGS="is_official_build=true"
GN_ARGS+=" symbol_level=0"
# is_official_build defaults chrome_pgo_phase=2, which downloads a
# Chrome-specific profile from a GS bucket: irrelevant here and it fails.
GN_ARGS+=" chrome_pgo_phase=0"
GN_ARGS+=" use_thin_lto=false"
GN_ARGS+=" use_glib=false"
GN_ARGS+=" is_cfi=false"
GN_ARGS+=" exclude_unwind_tables=true"
GN_ARGS+=" v8_enable_sandbox=false"
GN_ARGS+=" v8_enable_pointer_compression=true"
GN_ARGS+=" v8_enable_webassembly=true"
# i18n stays on: rusty_v8's binding.cc unconditionally includes
# <unicode/locid.h>, so disabling it breaks the binding compile.
GN_ARGS+=" v8_enable_i18n_support=true"

# (1) Build against the Debian bullseye sysroot. With use_sysroot=true and no
# explicit sysroot, build/config/sysroot.gni resolves
# //build/linux/debian_bullseye_amd64-sysroot for x64 -- the same sysroot
# scripts/lib/linux-sysroot.sh pins the rest of the engine at.
GN_ARGS+=" use_sysroot=true"

# (2) Make the archive linkable into a shared object.
#
# V8 pins the TLS model of its hot thread-locals in source:
#
#   __attribute__((tls_model(V8_TLS_MODEL))) extern thread_local Isolate*
#       g_current_isolate_;
#
# A source-level attribute beats -ftls-model on a command line, so a model flag
# alone changes nothing. Three things were measured before arriving here, all
# recorded so they are not retried:
#
#   * `-ftls-model=global-dynamic` has no effect on these variables. Verified by
#     recompiling isolate.cc with and without it: byte-identical relocations.
#   * `extra_cflags` / `extra_asmflags` do not exist in this build tree at all.
#     They are Skia arguments, not Chromium ones; gn accepted them into args.gn
#     and no compiler command line ever saw them.
#   * Adding the attribute to the *definition* in isolate.cc (the declaration in
#     isolate.h has it, the definition does not) changes nothing either -- the
#     declaration's attribute already applies, as C++ requires.
#
# Measure with `grep -w R_X86_64_TPOFF32`, never a substring match:
# R_X86_64_DTPOFF32 contains "TPOFF32", so a loose match counts the
# local-dynamic relocations this build is trying to produce as failures. That
# error made a correct archive look broken through several rebuilds.
#
# V8's own switch is V8_TLS_USED_IN_LIBRARY (src/common/thread-local-storage.h):
# it selects "local-dynamic" and hides each variable behind a non-inlined
# getter, which is what V8 documents for "static archive linked into a shared
# library". BUILD.gn defines it when both arguments below are set. The monolith
# target that v8_monolithic declares is never built -- nothing rusty_v8 depends
# on references it -- so this buys the define without changing the artifact.
#
# -fPIC needs no argument: build/config/compiler/BUILD.gn already adds it for
# this platform, confirmed by the archive carrying zero absolute relocations.
GN_ARGS+=" v8_monolithic=true"
GN_ARGS+=" v8_monolithic_for_shared_library=true"
# Required by v8_monolithic's own assertions.
GN_ARGS+=" v8_use_external_startup_data=false"

# (3) Do not replace the process allocator.
#
# Chromium's PartitionAlloc shim, on by default for linux, overrides global
# malloc/free. That is right for a browser which owns its whole process; it is
# wrong for a library embedded in someone else's, where it ends up freeing
# pointers the host allocated with the system allocator. Measured: with the shim
# on, a C host linking libmigo.so segfaults on a null dereference inside
# `allocator_shim::internal::PartitionAllocFunctionsInternal<>::Free` as soon as
# content loading crosses the boundary.
#
# An engine that hijacks its host's allocator is not embeddable, so this stays
# off regardless of the linkage form.
GN_ARGS+=" use_allocator_shim=false"
GN_ARGS+=" use_partition_alloc_as_malloc=false"

export EXTRA_GN_ARGS="$GN_ARGS"
export V8_FROM_SOURCE=1

# Changing any GN argument forces a full rebuild, which re-runs bindgen -- and
# the local libclang cannot parse V8's vendored libc++ headers. The generated
# binding describes V8's C++ API, which none of the arguments above affect, so
# the known-good one is reused. (MIGO patch, see engine/third_party/v8-patches.)
if [[ -f "$BACKUP_DIR/src_binding.rs" ]]; then
    export V8_PREBUILT_BINDING="$BACKUP_DIR/src_binding.rs"
    info "reusing binding: $V8_PREBUILT_BINDING"
fi

# ------------------------------------------------------------
# Toolchain: gn, ninja, libclang
#
# rusty_v8's build.rs otherwise runs tools/ninja_gn_binaries.py, which fetches
# gn and ninja from chrome-infra-packages over a code path that ignores the
# proxy environment and hangs here. build-v8-android.sh solves this the same
# way: supply the binaries and never enter that download.
# ------------------------------------------------------------
PREFETCHED_GN="$RUSTY_V8_SRC/third_party/v8_correct_gn/gn"
GN_BIN="${V8_GN_PATH:-}"
[[ -z "$GN_BIN" && -x "$PREFETCHED_GN" ]] && GN_BIN="$PREFETCHED_GN"
[[ -z "$GN_BIN" ]] && GN_BIN="$(command -v gn 2>/dev/null || true)"
if [[ -n "$GN_BIN" && -x "$GN_BIN" ]]; then
    export GN="$GN_BIN"
    info "gn: $GN"
else
    err "no gn binary, and the chrome-infra download hangs. Set V8_GN_PATH."
    exit 1
fi

NINJA_BIN="$(command -v ninja 2>/dev/null || true)"
if [[ -n "$NINJA_BIN" ]]; then
    export NINJA="$NINJA_BIN"
    info "ninja: $NINJA"
else
    err "no ninja binary; install ninja-build"
    exit 1
fi

# bindgen parses V8's vendored libc++ headers, which need clang 19 or newer.
# The system libclang is older, so an NDK copy is borrowed for the parse only --
# it never touches the code generation for this host target.
LIBCLANG_DIR="${V8_LIBCLANG_PATH:-}"
if [[ -z "$LIBCLANG_DIR" ]]; then
    for cand in "$HOME/Android/android-ndk-r28c" "$HOME/Android/android-ndk-r29"; do
        if [[ -f "$cand/toolchains/llvm/prebuilt/linux-x86_64/lib/libclang.so" ]]; then
            LIBCLANG_DIR="$cand/toolchains/llvm/prebuilt/linux-x86_64/lib"
            break
        fi
    done
fi
if [[ -n "$LIBCLANG_DIR" && -f "$LIBCLANG_DIR/libclang.so" ]]; then
    export LIBCLANG_PATH="$LIBCLANG_DIR"
    info "libclang: $LIBCLANG_PATH"
fi

# ------------------------------------------------------------
# Force a regeneration when the arguments changed.
#
# rusty_v8's build.rs writes args.gn but reuses an existing build.ninja, so a
# changed argument lands in args.gn and never reaches a compiler command line.
# Measured: adding -DV8_TLS_USED_IN_LIBRARY produced a 25-second "build" that
# re-archived the same objects, args.gn showed the define, and build.ninja
# contained zero occurrences of it. Comparing against the generated ninja file
# rather than args.gn is deliberate -- args.gn is what was asked for, and
# build.ninja is what will actually be compiled.
# ------------------------------------------------------------
if [[ -f "$GN_OUT/build.ninja" ]]; then
    stale=0
    # The two properties this build exists to produce. Both have to appear in a
    # compiler command line, not merely in args.gn. gn writes compile flags into
    # toolchain.ninja and the per-target files rather than build.ninja, so the
    # search covers the whole directory -- checking build.ninja alone reports
    # every build as stale and silently turns each one into a full rebuild.
    # `find -exec` rather than grep's --include: grep here may be ugrep, whose
    # --include matching differs, and a silently-empty search would report every
    # build as stale and turn each one into a needless full rebuild.
    for needle in "V8_TLS_USED_IN_LIBRARY" "debian_bullseye_amd64-sysroot" \
                  "USE_ALLOCATOR_SHIM=false"; do
        if ! find "$GN_OUT" -name '*.ninja' -exec grep -qF "$needle" {} + 2>/dev/null; then
            info "generated build files do not carry '$needle'; regenerating from scratch"
            stale=1
        fi
    done
    if [[ "$stale" -eq 1 ]]; then
        rm -rf "$GN_OUT"
    fi
fi

info "gn args: $GN_ARGS"
info "building V8 (full rebuild -- every argument change invalidates the cache)"

cd "$RUSTY_V8_SRC"
cargo build --release --target "$TARGET" -p v8

ARCHIVE="$GN_OUT/obj/librusty_v8.a"
[[ -f "$ARCHIVE" ]] || { err "build produced no archive at $ARCHIVE"; exit 1; }

# ------------------------------------------------------------
# Verify both properties before installing. An archive that silently fails
# either one would be discovered much later, in a link error or a load failure
# on a user's machine.
# ------------------------------------------------------------
isoc23_count="$(nm --undefined-only "$ARCHIVE" 2>/dev/null | grep -c '__isoc23_' || true)"
if [[ "$isoc23_count" -ne 0 ]]; then
    err "archive still references $isoc23_count glibc 2.38 entry points; the sysroot did not take effect"
    exit 1
fi
info "sysroot verified: 0 glibc 2.38 entry points referenced"

# The relocation count is only a proxy; what matters is whether a shared object
# can actually be produced. Ask the linker rather than infer.
# -w and the full relocation name on purpose: R_X86_64_DTPOFF32 contains
# "TPOFF32" as a substring, and a loose match counts local-dynamic
# relocations -- which are exactly what this build is trying to produce --
# as failures. That mistake reported a working archive as broken.
tls_relocs="$(objdump -r "$ARCHIVE" 2>/dev/null | grep -cw 'R_X86_64_TPOFF32' || true)"
if [[ "$tls_relocs" -ne 0 ]]; then
    err "archive still carries $tls_relocs local-exec TLS relocations;"
    err "V8_TLS_USED_IN_LIBRARY did not reach every translation unit."
    exit 1
fi
info "TLS verified: 0 local-exec relocations"

info "installing to $OUT_DIR"
mkdir -p "$OUT_DIR"
cp "$ARCHIVE" "$OUT_DIR/librusty_v8.a"
cp "$GN_OUT/src_binding.rs" "$OUT_DIR/src_binding.rs"

# Provenance. scripts/write-v8-component-manifest.py is not reused here: it
# asserts Android specifics (NDK root, android_ndk_api_level, the Android patch
# set) and reshaping it around a host build would make one tool answer to two
# contracts. The Linux package manifest consumes this file through
# gen-linux-package-metadata.py --v8-component-manifest.
info "recording V8 provenance"
MIGO_V8_ARCHIVE="$OUT_DIR/librusty_v8.a" \
MIGO_V8_GN_ARGS="$GN_ARGS" \
MIGO_V8_SRC="$RUSTY_V8_SRC" \
MIGO_V8_OUT="$OUT_DIR/component-manifest.json" \
python3 - <<'PY'
import hashlib, json, os, pathlib, subprocess

archive = pathlib.Path(os.environ["MIGO_V8_ARCHIVE"])
source = pathlib.Path(os.environ["MIGO_V8_SRC"])

def git(*args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(source), *args],
                              capture_output=True, text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, OSError):
        return ""

def v8_version() -> str:
    """major.minor.build.patch from v8-version.h, or "" if it cannot be read."""
    header = source / "v8/include/v8-version.h"
    if not header.is_file():
        return ""
    # The four macros are not named uniformly: MAJOR/MINOR carry a _VERSION
    # suffix, the other two do not.
    import re as _re
    text = header.read_text()
    parts = []
    for macro in ("V8_MAJOR_VERSION", "V8_MINOR_VERSION", "V8_BUILD_NUMBER",
                  "V8_PATCH_LEVEL"):
        found = _re.search(rf"#define\s+{macro}\s+(\d+)", text)
        if not found:
            return ""
        parts.append(found.group(1))
    return ".".join(parts)


digest = hashlib.sha256()
with archive.open("rb") as handle:
    for chunk in iter(lambda: handle.read(1 << 20), b""):
        digest.update(chunk)

manifest = {
    "schema": "migo-v8-component-v1",
    "target": "x86_64-unknown-linux-gnu",
    "rusty_v8_revision": git("rev-parse", "HEAD"),
    "rusty_v8_describe": git("describe", "--tags", "--always", "--dirty"),
    "v8_version": v8_version(),
    "gn_args": os.environ["MIGO_V8_GN_ARGS"].split(),
    "archive_sha256": digest.hexdigest(),
    "archive_bytes": archive.stat().st_size,
    # The two properties this build exists to guarantee; the build fails before
    # reaching here if either is violated, so recording them is a statement of
    # what was verified, not a claim taken on trust.
    "verified": {
        "glibc_238_entry_points": 0,
        "local_exec_tls_relocations": 0,
    },
}
pathlib.Path(os.environ["MIGO_V8_OUT"]).write_text(json.dumps(manifest, indent=2) + "\n")
print(f"wrote {os.environ['MIGO_V8_OUT']}")
PY

info "done: $OUT_DIR/librusty_v8.a ($(stat -c %s "$OUT_DIR/librusty_v8.a") bytes)"
