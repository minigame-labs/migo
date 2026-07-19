#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The packaging gate points this at the staged package so the contract
# tests the headers a consumer actually receives, not the source tree.
INCLUDE_DIR="${MIGO_INCLUDE_DIR:-$ROOT/include}"
CC_BIN="${CC:-cc}"
CXX_BIN="${CXX:-c++}"
MODE="${1:---all}"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

compile_c() {
    local source="$1"
    local output="$2"
    "$CC_BIN" -std=c11 -Wall -Wextra -Werror -pedantic \
        -I"$INCLUDE_DIR" -c "$source" -o "$output"
}

compile_cpp() {
    local source="$1"
    local output="$2"
    "$CXX_BIN" -std=c++17 -Wall -Wextra -Werror -pedantic \
        -I"$INCLUDE_DIR" -c "$source" -o "$output"
}

compile_header_standalone() {
    local header="$1"
    local stem="${header//\//_}"
    "$CC_BIN" -x c -std=c11 -Wall -Wextra -Werror -pedantic \
        -I"$INCLUDE_DIR" -include "$header" -c /dev/null \
        -o "$TMP_ROOT/${stem}.o"
    "$CXX_BIN" -x c++ -std=c++17 -Wall -Wextra -Werror -pedantic \
        -I"$INCLUDE_DIR" -include "$header" -c /dev/null \
        -o "$TMP_ROOT/${stem}_cpp.o"
}

compile_core() {
    local header
    for header in migo/types.h migo/surface.h migo/session.h migo/input.h migo/migo.h; do
        compile_header_standalone "$header"
    done
    compile_c "$ROOT/tests/c_abi/core_contract.c" "$TMP_ROOT/core_contract.o"
    compile_cpp "$ROOT/tests/c_abi/core_contract.cc" "$TMP_ROOT/core_contract_cpp.o"
}

compile_platforms() {
    local header
    for header in \
        migo/platform/android.h \
        migo/platform/win32.h \
        migo/platform/winui.h \
        migo/platform/macos.h \
        migo/platform/x11.h \
        migo/platform/wayland.h \
        migo/platform/openharmony.h; do
        compile_header_standalone "$header"
    done
    compile_c "$ROOT/tests/c_abi/platform_contract.c" "$TMP_ROOT/platform_contract.o"
    compile_cpp "$ROOT/tests/c_abi/platform_contract.cc" "$TMP_ROOT/platform_contract_cpp.o"
}

require_literal() {
    local file="$1"
    local literal="$2"
    local reason="$3"
    if [[ ! -f "$file" ]] || ! grep -Fq "$literal" "$file"; then
        echo "C ABI contract missing: $reason ($file)" >&2
        return 1
    fi
}

require_regex() {
    local file="$1"
    local pattern="$2"
    local reason="$3"
    if [[ ! -f "$file" ]] || ! grep -Eq "$pattern" "$file"; then
        echo "C ABI contract missing: $reason ($file)" >&2
        return 1
    fi
}

check_repository_integration() {
    require_regex "$ROOT/.github/workflows/pr-ci.yml" \
        '^[[:space:]]+bash scripts/test-c-abi-surface-candidate\.sh$' \
        "active PR quality gate"
    require_regex "$ROOT/.github/workflows/release.yml" \
        '^[[:space:]]+bash scripts/test-c-abi-surface-candidate\.sh$' \
        "active release quality gate"
    require_regex "$ROOT/.github/workflows/pr-ci.yml" \
        '^[[:space:]]+CC=clang CXX=clang\+\+ bash scripts/test-c-abi-surface-candidate\.sh$' \
        "Clang PR quality gate"
    require_regex "$ROOT/.github/workflows/release.yml" \
        '^[[:space:]]+CC=clang CXX=clang\+\+ bash scripts/test-c-abi-surface-candidate\.sh$' \
        "Clang release quality gate"
    require_literal "$ROOT/.github/workflows/pr-ci.yml" \
        "armv7a-linux-androideabi26-clang" "API 26 ARMv7 PR layout gate"
    require_literal "$ROOT/.github/workflows/release.yml" \
        "armv7a-linux-androideabi26-clang" "API 26 ARMv7 release layout gate"
    require_regex "$ROOT/.github/workflows/c-abi-candidate.yml" \
        '^[[:space:]]+bash scripts/test-c-abi-surface-candidate\.sh$' \
        "candidate docs/header PR gate"
    require_regex "$ROOT/.github/workflows/c-abi-candidate.yml" \
        '^[[:space:]]+CC=clang CXX=clang\+\+ bash scripts/test-c-abi-surface-candidate\.sh$' \
        "candidate docs/header Clang gate"
    require_literal "$ROOT/include/migo/README.md" "design candidate" \
        "candidate status documentation"
    require_literal "$ROOT/include/migo/README.md" \
        "must not wait for another turn of the host dispatcher" \
        "detach dispatcher-independence invariant"
    require_literal "$ROOT/include/migo/README.md" \
        "Every reserved field must remain zero" \
        "reserved-field compatibility rule"
    require_literal "$ROOT/include/migo/README.md" \
        "can be installed only once" "callback replacement lifetime rule"
    require_literal "$ROOT/include/migo/README.md" \
        '`MigoSurfaceDescriptor.platform_descriptor_size` deliberately duplicates' \
        "platform descriptor cross-check rationale"
    require_literal "$ROOT/docs/multiplatform-architecture.md" \
        "include/migo/migo.h" "architecture header reference"
    require_literal "$ROOT/docs/multiplatform-architecture.md" \
        "MIGO_C_ABI_HAS_RUNTIME" "architecture no-runtime marker"
}

case "$MODE" in
    --core)
        compile_core
        ;;
    --platforms)
        compile_platforms
        ;;
    --all)
        compile_core
        compile_platforms
        check_repository_integration
        ;;
    *)
        echo "usage: $0 [--core|--platforms|--all]" >&2
        exit 2
        ;;
esac

echo "C ABI Surface candidate contract: PASS ($MODE)"
