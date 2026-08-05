#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORM="$ROOT/engine/crates/platform/src/linux"
CAPI_SURFACE="$ROOT/engine/crates/capi/src/surface.rs"

fail() {
    echo "Owned X11 connection contract failed: $*" >&2
    exit 1
}

if ! command -v rg >/dev/null 2>&1; then
    echo "Owned X11 connection contract could not run: ripgrep (rg) is not installed." >&2
    exit 127
fi

require_literal() {
    local file="$1"
    local literal="$2"
    local reason="$3"
    if [[ ! -f "$file" ]] || ! grep -Fq "$literal" "$file"; then
        fail "$reason ($file)"
    fi
}

require_pattern() {
    local file="$1"
    local pattern="$2"
    local reason="$3"
    if [[ ! -f "$file" ]] || ! rg -Uq "$pattern" "$file"; then
        fail "$reason ($file)"
    fi
}

CONNECTION="$PLATFORM/x11_connection.rs"
PRESENTER="$PLATFORM/presenter.rs"
UPLOAD="$ROOT/engine/crates/graphics/src/upload_thread.rs"

require_literal "$CONNECTION" "pub(super) struct X11RenderConnection" \
    "private X11 render-connection owner is missing"
require_literal "$CONNECTION" 'symbol(&library, b"XOpenDisplay\0")' \
    "owned connection does not resolve XOpenDisplay"
require_literal "$CONNECTION" 'symbol(&library, b"XCloseDisplay\0")' \
    "owned connection does not resolve XCloseDisplay"
require_literal "$CONNECTION" "api.close_display(self.display);" \
    "owned connection has no RAII close path"
require_literal "$PRESENTER" "pub struct LinuxX11Context" \
    "session-scoped X11 context is missing"
require_literal "$PRESENTER" "LinuxDisplayTarget::X11(_) => EglConcurrency::RenderThreadOnly" \
    "X11 provider is not render-thread-only"
require_literal "$PRESENTER" "fn x11_context_binds_identity_surface_and_factory_to_one_owned_connection" \
    "X11 identity/factory/concurrency test is missing"
require_literal "$UPLOAD" \
    "provider.concurrency() == EglConcurrency::SharedContexts" \
    "upload worker does not fail closed on provider concurrency"

if rg -n "XInitThreads" \
    "$ROOT/engine/crates/platform/src/linux" \
    "$ROOT/engine/tools/player" \
    "$ROOT/tests/c_host/linux" \
    "$ROOT/include/migo"; then
    fail "host-side XInitThreads precondition remains"
fi

context_build_line="$(
    rg -n 'build_target\(descriptor, existing_platform_context\.as_ref\(\)\)' \
        "$CAPI_SURFACE" | cut -d: -f1
)"
first_lease_line="$(
    rg -n 'let lease = match lease_surface_tracked\(' \
        "$CAPI_SURFACE" | cut -d: -f1
)"
if [[ -z "$context_build_line" || -z "$first_lease_line" ]] \
    || (( context_build_line >= first_lease_line )); then
    fail "C ABI does not reuse/validate the X11 context before Surface lease"
fi

require_literal "$ROOT/include/migo/platform/x11.h" \
    "borrows display only for this attach call" \
    "public header does not state the synchronous Display* borrow"
require_pattern "$ROOT/include/migo/platform/x11.h" \
    'The host must keep window\s*\n\s*\*\s*valid until the release observer reaches\s*\n\s*\*\s*MIGO_SURFACE_RELEASE_RELEASED\.' \
    "public header does not retain the host Window through RELEASED"
require_literal "$ROOT/include/migo/README.md" \
    'Migo opens a private render connection' \
    "ABI guide does not explain private X11 connection ownership"
require_literal "$ROOT/engine/tools/player/src/main.rs" \
    "LinuxX11Context::open(window.display())" \
    "Rust player bypasses the shipping X11 context path"
require_literal "$ROOT/.github/workflows/pr-ci.yml" \
    "bash scripts/test-x11-owned-connection-contract.sh" \
    "PR CI does not enforce the owned X11 connection contract"
require_literal "$ROOT/.github/workflows/release.yml" \
    "bash scripts/test-x11-owned-connection-contract.sh" \
    "release CI does not enforce the owned X11 connection contract"

echo "Owned X11 connection contract: PASS"
