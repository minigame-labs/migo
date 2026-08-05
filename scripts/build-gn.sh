#!/usr/bin/env bash
# ============================================================
# Build the pinned `gn` that the V8 builds require.
# Location: scripts/build-gn.sh
#
# V8 14.5 needs gn >= 2315 for `path_exists()`, and rusty_v8's own
# ninja_gn_binaries.py download uses http.client, which ignores https_proxy and
# times out in this network. So gn is built from source at a pinned revision and
# installed where scripts/build-v8-*.sh probe for it.
#
# The revision and the required patches come from
# contracts/artifact-manifest/android-v8.lock.json. Nothing here restates them.
#
# The installed binary is accompanied by a receipt recording the revision, the
# sha256 of every patch applied, and the sha256 of the binary itself. `gn
# --version` cannot stand in for that: it prints a commit *position* derived from
# `git describe HEAD`, with no dirty marker, so a gn built from the pinned commit
# without the required patch -- or with extra local edits -- reports exactly the
# same string as the intended one.
#
# Usage:
#   ./scripts/build-gn.sh [--src <gn-checkout>] [--prefix <install-dir>]
#
# Defaults, both derived from this repository's location rather than hardcoded:
#   --src     <repo-parent>/gn
#   --prefix  $RUSTY_V8_SRC/third_party/v8_correct_gn
#
# Env:
#   CXX  host C++ compiler. Must accept the C++ standard gn compiles itself with,
#        which the Android NDK's clang 12 does not -- and the NDK precedes the
#        host compiler on PATH in this environment, so leaving this to gn's own
#        `clang++` default fails.
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_PARENT="$(cd "$PROJECT_ROOT/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
GN_PATCH_DIR="$ENGINE_ROOT/third_party/gn-patches"
V8_BUILD_LOCK="$PROJECT_ROOT/contracts/artifact-manifest/android-v8.lock.json"

RUSTY_V8_SRC="${RUSTY_V8_SRC:-$REPO_PARENT/rusty_v8_src}"
GN_SRC="$REPO_PARENT/gn"
GN_PREFIX=""

info() { echo -e "\033[0;36m[gn] $*\033[0m"; }
ok()   { echo -e "\033[0;32m[gn] $*\033[0m"; }
err()  { echo -e "\033[0;31m[gn] $*\033[0m" >&2; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --src)    GN_SRC="$2"; shift 2 ;;
        --prefix) GN_PREFIX="$2"; shift 2 ;;
        *) err "unknown argument: $1"; exit 1 ;;
    esac
done
[[ -n "$GN_PREFIX" ]] || GN_PREFIX="$RUSTY_V8_SRC/third_party/v8_correct_gn"

# shellcheck source=scripts/lib/v8-patch-apply.sh
source "$SCRIPT_DIR/lib/v8-patch-apply.sh"
# shellcheck source=scripts/lib/gn-pin.sh
source "$SCRIPT_DIR/lib/gn-pin.sh"

gn_pin_read "$V8_BUILD_LOCK"
info "pinned gn: version $GN_PIN_VERSION revision $GN_PIN_REVISION"

[[ -d "$GN_SRC/.git" ]] || {
    err "no gn checkout at $GN_SRC"
    err "run: git clone https://gn.googlesource.com/gn $GN_SRC"
    exit 1
}

# The checkout is not necessarily owned by this user; -c keeps that decision to
# this invocation instead of writing it into the user's global git config.
git_gn() { git -c "safe.directory=$GN_SRC" -C "$GN_SRC" "$@"; }

head_revision="$(git_gn rev-parse HEAD)"
if [[ "$head_revision" != "$GN_PIN_REVISION" ]]; then
    err "gn checkout is at $head_revision but the lock pins $GN_PIN_REVISION"
    err "run: git -C $GN_SRC fetch origin && git -C $GN_SRC checkout $GN_PIN_REVISION"
    exit 1
fi

info "applying pinned gn patches"
declare -a patch_globs=()
for patch_id in "${GN_PIN_PATCHES[@]}"; do
    patch_globs+=("$patch_id.patch")
    v8_require_patch "$GN_SRC" "$GN_PATCH_DIR" "$patch_id.patch" || exit 1
done

# Applying the declared patches proves each of them landed; it does not prove
# nothing else did. A gn carrying an extra local edit builds and reports the
# pinned version, so the difference would reach the archive unrecorded.
info "checking the checkout is HEAD plus exactly the declared patches"
v8_assert_tree_is_exactly_patched "$GN_SRC" "$GN_PATCH_DIR" "${patch_globs[@]}" || {
    err "the gn checkout carries changes the pinned patches do not account for"
    exit 1
}

# gn compiles itself with a recent C++ standard. Probe the compiler rather than
# matching version numbers, so the check states the requirement itself.
HOST_CXX="${CXX:-}"
if [[ -z "$HOST_CXX" ]]; then
    for candidate in /usr/bin/g++ /usr/bin/clang++ g++ clang++; do
        command -v "$candidate" >/dev/null 2>&1 && { HOST_CXX="$candidate"; break; }
    done
fi
[[ -n "$HOST_CXX" ]] || { err "no host C++ compiler found; set CXX"; exit 1; }
GN_CXX_STANDARD="$(sed -n "s/.*'-std=\(c++[0-9a-z]*\)'.*/\1/p" "$GN_SRC/build/gen.py" \
                   | sort -u | tail -1)"
[[ -n "$GN_CXX_STANDARD" ]] || { err "cannot tell which C++ standard gn wants"; exit 1; }
probe_dir="$(mktemp -d)"
printf 'int main() { return 0; }\n' > "$probe_dir/probe.cc"
if ! "$HOST_CXX" "-std=$GN_CXX_STANDARD" -fsyntax-only "$probe_dir/probe.cc" >/dev/null 2>&1; then
    rm -rf "$probe_dir"
    err "$HOST_CXX does not accept -std=$GN_CXX_STANDARD, which gn compiles itself with"
    err "the Android NDK's clang 12 precedes the host compiler on PATH here;"
    err "set CXX to a newer compiler, for example CXX=/usr/bin/g++"
    exit 1
fi
rm -rf "$probe_dir"
info "host compiler: $HOST_CXX (accepts -std=$GN_CXX_STANDARD)"

info "generating the gn build"
# From scratch, not incrementally. gn's `out*` directories are gitignored, so
# `v8_assert_tree_is_exactly_patched` cannot see bytes that live there: an
# existing out/gn with a recent mtime -- a hand-built or copied binary, say --
# would leave ninja with nothing to do, and this script would install it and write
# it a matching receipt, certifying a binary it never produced. The receipt must
# only ever describe bytes this run compiled.
GN_OUT="$GN_SRC/out"
rm -rf "$GN_OUT"
( cd "$GN_SRC" && CXX="$HOST_CXX" python3 build/gen.py --out-path out )
info "compiling gn"
CXX="$HOST_CXX" ninja -C "$GN_OUT" gn
[[ -x "$GN_OUT/gn" ]] || { err "ninja produced no gn at $GN_OUT/gn"; exit 1; }

built_version="$("$GN_OUT/gn" --version)"
gn_pin_assert_version "$built_version" || exit 1

mkdir -p "$GN_PREFIX"
install -m 0755 "$GN_OUT/gn" "$GN_PREFIX/gn"
gn_pin_write_receipt "$GN_PREFIX/gn" "$GN_PATCH_DIR" || exit 1
ok "installed gn $built_version -> $GN_PREFIX/gn"
ok "receipt -> $GN_PREFIX/gn-receipt.json"
