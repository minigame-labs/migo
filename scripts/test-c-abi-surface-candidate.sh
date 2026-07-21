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
    # The old-client lane: a translation unit carrying a previous header's shape,
    # asserting it is still a byte-exact prefix of the current one. An inserted
    # field would keep the size a short client announces and change what every
    # byte after it means, which no size check can catch.
    compile_c "$ROOT/tests/c_abi/old_client_contract.c" "$TMP_ROOT/old_client_contract.o"
    # The output-side mirror: library-written structs must grow append-only too,
    # so a v1 client's buffer is never overrun. Independent redeclaration catches
    # a same-type field swap that size and header pins cannot see.
    compile_c "$ROOT/tests/c_abi/old_client_outbound_contract.c" \
        "$TMP_ROOT/old_client_outbound_contract.o"
}

# The ILP32 lane.
#
# Every layout assertion in tests/c_abi is written twice, once per pointer
# width, but until 2026-07-21 only the LP64 half had ever been compiled: every
# lane ran on an LP64 host, so `#elif UINTPTR_MAX == UINT32_MAX` was dead
# source. It had been wrong since the commit that appended `on_request_frame`
# -- that commit updated the LP64 size and not the ILP32 one, and the
# soft-keyboard callbacks were then appended on top of the wrong base. A
# 32-bit compile is what makes that half real.
#
# `-ffreestanding` is what keeps this cheap: the lanes need only stdint.h and
# stddef.h, which the compiler supplies itself, so this needs a multilib
# compiler but not a 32-bit libc.
compile_c_ilp32() {
    local source="$1"
    "$CC_BIN" -m32 -ffreestanding -std=c11 -Wall -Wextra -Werror -pedantic \
        -I"$INCLUDE_DIR" -c "$source" -o "$2"
}

ilp32_available() {
    echo 'int main(void){return 0;}' > "$TMP_ROOT/probe32.c"
    "$CC_BIN" -m32 -ffreestanding -c "$TMP_ROOT/probe32.c" \
        -o "$TMP_ROOT/probe32.o" 2>/dev/null
}

compile_ilp32() {
    if ! ilp32_available; then
        # Reported, never silent. A skipped lane that prints nothing is
        # indistinguishable from a lane that passed, which is how the wrong
        # size survived this long.
        echo "C ABI ILP32 lane: SKIPPED ($CC_BIN cannot target -m32; install a multilib compiler)" >&2
        if [[ "${MIGO_ABI_REQUIRE_ILP32:-0}" == "1" ]]; then
            echo "C ABI ILP32 lane: required by MIGO_ABI_REQUIRE_ILP32=1" >&2
            return 1
        fi
        return 0
    fi
    local source
    for source in core_contract old_client_contract old_client_outbound_contract \
                  platform_contract; do
        compile_c_ilp32 "$ROOT/tests/c_abi/$source.c" "$TMP_ROOT/${source}_ilp32.o"
    done
    echo "C ABI ILP32 lane: PASS"
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

# Every migo_* the headers declare must be exported by the Rust implementation,
# and every export must be declared. Compiling the headers proves they are valid
# C; it does not prove anything links. A declaration with no export is a link
# error in the host's build, discovered by the host -- which is the worst place
# to discover it.
check_export_parity() {
    python3 - "$INCLUDE_DIR" "$ROOT/engine/crates/capi" <<'PY'
import pathlib
import re
import sys

include_dir, capi_dir = (pathlib.Path(a) for a in sys.argv[1:3])

# Comments name entry points when explaining them, so they must go before any
# identifier is believed. Prose is not a declaration.
block_comment = re.compile(r"/\*.*?\*/", re.S)
line_comment = re.compile(r"//[^\n]*")
call = re.compile(r"\bmigo_[A-Za-z0-9_]*\s*\(")

declared = set()
for header in sorted(include_dir.rglob("*.h")):
    text = line_comment.sub("", block_comment.sub("", header.read_text()))
    declared.update(m.group(0)[:-1].strip() for m in call.finditer(text))

exported = set()
export = re.compile(
    r'#\[unsafe\(no_mangle\)\]\s*(?:#\[[^\]]*\]\s*)*'
    r'pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(migo_[A-Za-z0-9_]*)'
)
for source in sorted(capi_dir.rglob("*.rs")):
    exported.update(export.findall(source.read_text()))

missing = sorted(declared - exported)
undeclared = sorted(exported - declared)
if missing:
    print("declared in include/ but not exported by Rust: " + ", ".join(missing), file=sys.stderr)
if undeclared:
    print("exported by Rust but not declared in include/: " + ", ".join(undeclared), file=sys.stderr)
if missing or undeclared:
    sys.exit(1)
print(f"C ABI export parity: {len(declared)} entry points agree")
PY
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
    --parity)
        check_export_parity
        ;;
    --ilp32)
        compile_ilp32
        ;;
    --all)
        compile_core
        compile_platforms
        compile_ilp32
        check_export_parity
        check_repository_integration
        ;;
    *)
        echo "usage: $0 [--core|--platforms|--ilp32|--parity|--all]" >&2
        exit 2
        ;;
esac

echo "C ABI Surface candidate contract: PASS ($MODE)"
