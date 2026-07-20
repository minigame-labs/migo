#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATES="$ROOT/engine/crates"

fail() {
    echo "SurfaceAttachment contract failed: $*" >&2
    exit 1
}

# A gate whose tool is missing must say so, not report the invariant it was
# unable to check.
#
# `require_multiline_regex` runs `rg` inside an `if !` condition, where a
# missing binary exits 127 and reads as "the pattern was not found" -- so on a
# runner without ripgrep this script announced that SurfaceDestroyed is not
# generation-tagged. It said that for weeks on CI. The invariant was fine; the
# gate was checking nothing and blaming the code, which is worse than not
# running at all, because it sends whoever reads it hunting for a bug that does
# not exist.
if ! command -v rg >/dev/null 2>&1; then
    echo "SurfaceAttachment contract could not run: ripgrep (rg) is not installed." >&2
    echo "This is a missing tool, NOT a contract violation. Install ripgrep and re-run." >&2
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

require_multiline_regex() {
    local file="$1"
    local pattern="$2"
    local reason="$3"
    if [[ ! -f "$file" ]] || ! rg -qU "$pattern" "$file"; then
        fail "$reason ($file)"
    fi
}

require_active_run() {
    local file="$1"
    local command="$2"
    if [[ ! -f "$ROOT/$file" ]] || ! awk -v command="$command" '
        {
            line = $0
            sub(/^[[:space:]]+/, "", line)
            sub(/[[:space:]]+$/, "", line)
            sub(/^run:[[:space:]]*/, "", line)
            if (line == command) found = 1
        }
        END { exit !found }
    ' "$ROOT/$file"; then
        fail "missing active workflow command '$command' ($file)"
    fi
}

if rg -n \
    'surface_epoch|DESTROY_EPOCHS|bump_destroy_epoch|current_destroy_epoch' \
    "$CRATES"; then
    fail "legacy destroy-epoch wiring remains"
fi

if rg -n \
    'raw_window_handle|RawWindowHandle|AndroidNdkWindowHandle|onscreen_window_from_surface|last_window' \
    "$CRATES"; then
    fail "legacy raw-window or naked-window recovery boundary remains"
fi
# Choosing which EGL shared object to dlopen at runtime is a platform-provider
# concern: it belongs in that platform's presenter and must never leak into
# graphics/core/js-runtime, which stay backend-agnostic. Presenters are listed
# one per platform on purpose — a new backend has to add itself here, and that
# review is the point of the gate. Build scripts are exempt because they
# configure link flags at compile time, not runtime implementation selection.
if rg -n 'libEGL\.so' "$CRATES" \
    --glob '!**/platform/android/presenter.rs' \
    --glob '!**/platform/desktop/presenter.rs' \
    --glob '!**/build.rs'; then
    fail "EGL implementation selection escaped the platform providers"
fi

require_multiline_regex "$CRATES/shared/protocol/host_cmd.rs" \
    'SurfaceDestroyed[[:space:]]*\{[^}]*generation:[[:space:]]*SurfaceGeneration' \
    "Host SurfaceDestroyed command is not generation-tagged"
require_multiline_regex "$CRATES/shared/protocol/render_cmd.rs" \
    'SurfaceDestroyed[[:space:]]*\{[[:space:]]*generation:[[:space:]]*SurfaceGeneration' \
    "render SurfaceDestroyed command is not generation-tagged"
require_multiline_regex "$CRATES/shared/protocol/render_cmd.rs" \
    'RecreateOnscreen[[:space:]]*\{[[:space:]]*lease:[[:space:]]*SurfaceLease' \
    "onscreen recreation does not carry a SurfaceLease"
require_literal "$CRATES/core/runtime/registry.rs" \
    "surface_gate: Arc<SurfaceGenerationGate>" \
    "Host registry is missing the queue-independent generation gate"
require_literal "$CRATES/core/runtime/registry.rs" \
    "surface_gate.retire_current();" \
    "shutdown does not retire the current Surface before render join"
require_literal "$CRATES/core/services/render.rs" \
    "attachment: SurfaceAttachmentSlot" \
    "Host render service is missing its unique attachment slot"
require_literal "$CRATES/graphics/render_thread.rs" \
    "let mut render_binding = RenderSurfaceBinding::new();" \
    "render thread is missing its retained Surface binding"
require_literal "$CRATES/graphics/canvas/handler.rs" \
    "RecreateOnscreen must be preflighted by the render thread" \
    "CanvasHandler can bypass generation preflight"
require_literal "$CRATES/graphics/surface_binding.rs" \
    "pub(crate) fn clear_after_egl_teardown" \
    "native Surface resource release is not ordered after EGL teardown"
require_multiline_regex "$CRATES/graphics/render_thread.rs" \
    'binding[[:space:]]*\.[[:space:]]*preflight\(&lease\)' \
    "Surface generation is not rejected before presenter preparation"
require_literal "$CRATES/graphics/render_thread.rs" \
    "cm.is_surface_recovery_ready()" \
    "context recovery is not gated on a fully installed prepared target"
require_literal "$CRATES/graphics/render_thread.rs" \
    ".validate_prepared(prepared.as_ref())" \
    "prepared presenter backend is not revalidated inside installation"
require_literal "$CRATES/graphics/canvas/manager/mod.rs" \
    "installed_surface: Option<PreparedEglSurfaceRef>" \
    "CanvasManager does not retain the prepared presentation target"
require_literal "$CRATES/graphics/canvas/manager/mod.rs" \
    "drawing_buffer: Option<drawing_buffer::DrawingBuffer>" \
    "partial onscreen EGL ownership does not keep the preserved DrawingBuffer paired with its context"
require_literal "$CRATES/graphics/canvas/manager/mod.rs" \
    "self.preserved_drawing_buffer = pending.drawing_buffer.take()" \
    "partial onscreen cleanup does not restore the preserved context/DB pair"
require_literal "$CRATES/graphics/canvas/manager/types.rs" \
    "Window," \
    "window SurfaceKind still carries a platform-native integer"
require_literal "$CRATES/graphics/canvas/manager/egl_ops.rs" \
    "struct InitializedDisplayGuard" \
    "initialized EGL display has no early-return teardown guard"
require_literal "$CRATES/graphics/canvas/manager/egl_ops.rs" \
    "struct ContextCleanupGuard" \
    "partial pbuffer creation has no EGLContext teardown guard"
require_literal "$CRATES/graphics/canvas/manager/egl_ops.rs" \
    "pub(super) struct EglRuntime" \
    "CanvasManager construction has no owned EGL display fallback"
require_literal "$CRATES/graphics/upload_thread.rs" \
    "provider.backend_id() != expected_backend" \
    "upload EGL dispatch does not fail closed on provider identity mismatch"

ANDROID_PRESENTER="$CRATES/platform/android/presenter.rs"
require_literal "$ANDROID_PRESENTER" \
    "pub struct AndroidEglProvider" \
    "Android system-EGL provider is missing"
require_literal "$ANDROID_PRESENTER" \
    "pub struct AndroidEglSurfaceFactory" \
    "Android EGL surface factory is missing"
require_literal "$ANDROID_PRESENTER" \
    "pub struct AndroidPreparedSurface" \
    "Android prepared surface target is missing"
require_literal "$ANDROID_PRESENTER" \
    'const ANDROID_EGL_LIBRARY: &str = "libEGL.so";' \
    "Android provider must select the system EGL library explicitly"
require_literal "$ANDROID_PRESENTER" \
    "GraphicsPlatform::try_new" \
    "Android presenter bundle is not validated"
require_multiline_regex "$ANDROID_PRESENTER" \
    'as_any\(\)[[:space:]]*\.[[:space:]]*downcast_ref::<AndroidSurfaceWrapper>\(\)' \
    "Android presenter does not fail-closed downcast the platform Surface"
if [[ -f "$ANDROID_PRESENTER" ]] && rg -n 'ANativeWindow_(acquire|release)' "$ANDROID_PRESENTER"; then
    fail "prepared Android presenter target must remain non-owning ($ANDROID_PRESENTER)"
fi
require_literal "$CRATES/platform/android/jni/inbound.rs" \
    "android_graphics_platform()" \
    "Android bootstrap does not inject its matched graphics platform"

require_literal "$ROOT/include/migo/types.h" \
    "#define MIGO_C_ABI_HAS_RUNTIME 0" \
    "C ABI candidate must remain compile-only"
require_literal "$CRATES/platform/android/jni/profile_contract.rs" \
    '("updateSurface", "(ILjava/lang/Object;II)V")' \
    "Android updateSurface JNI descriptor changed"
require_literal "$CRATES/platform/android/jni/profile_contract.rs" \
    '("onSurfaceDestroyed", "(I)V")' \
    "Android onSurfaceDestroyed JNI descriptor changed"

require_active_run .github/workflows/pr-ci.yml \
    'bash scripts/test-surface-attachment-contract.sh'
require_active_run .github/workflows/release.yml \
    'bash scripts/test-surface-attachment-contract.sh'

(
    cd "$ROOT/engine"
    cargo test -p shared surface::attachment --lib --locked --offline
)

echo "SurfaceAttachment lifecycle contract: PASS"
