#!/usr/bin/env bash
# Host-only R9 Worker-snapshot identity, bootstrap, and artifact-routing contract.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE="$ROOT/engine"
JS_RUNTIME="$ENGINE/crates/runtime-v8"
FINGERPRINT="$ROOT/scripts/lib/snapshot-fingerprint.sh"
WRITE_MANIFEST="$ROOT/scripts/write-snapshot-manifest.sh"
FRESHNESS="$ROOT/scripts/check-snapshot-freshness.sh"
GENERATOR="$ROOT/scripts/gen-snapshot.sh"
ANDROID_SO="$ROOT/scripts/build-android-so.sh"
ANDROID_SO_PS="$ROOT/scripts/build-android-so.ps1"
AAR="$ROOT/scripts/build-aar.sh"
AAR_PS="$ROOT/scripts/build-aar.ps1"
GRADLE="$ROOT/platforms/android/library/build.gradle"
WORKFLOW="$ROOT/.github/workflows/build-snapshot.yml"
WORKER_JS="$JS_RUNTIME/src/99_worker_main.js"
WORKER_RS="$JS_RUNTIME/src/worker/mod.rs"
TEST_TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/migo-r9-contract.XXXXXX")"
trap 'rm -rf "$TEST_TMP_ROOT"' EXIT

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

require_literal() {
    local file="$1"
    local literal="$2"
    local description="$3"
    grep -Fq -- "$literal" "$file" || fail "$description"
}

expect_rejection() {
    local expected="$1"
    shift
    local output
    if output=$("$@" 2>&1); then
        fail "command unexpectedly succeeded: $*"
    fi
    [[ "$output" == *"$expected"* ]] \
        || fail "command did not report '$expected': $*; output=$output"
}

echo "[1/5] checking kind-bound snapshot identity"
require_literal "$FINGERPRINT" "SNAPSHOT_SCHEMA_VERSION=3" \
    "snapshot manifest schema must advance to 3"
require_literal "$FINGERPRINT" "snapshot_kind" \
    "shell fingerprint helpers must carry snapshot_kind"
require_literal "$JS_RUNTIME/build_snapshot.rs" "snapshot_kind" \
    "Rust snapshot identity must carry snapshot_kind"
require_literal "$WRITE_MANIFEST" 'SNAPSHOT_KIND="${4:-host}"' \
    "manifest writer must default its fourth kind argument to host"
for script in "$GENERATOR" "$FRESHNESS"; do
    require_literal "$script" "--snapshot-kind" \
        "$(basename "$script") lacks --snapshot-kind"
    require_literal "$script" "SNAPSHOT-worker-" \
        "$(basename "$script") lacks the isolated Worker filename"
done
expect_rejection "invalid snapshot kind" \
    bash "$FRESHNESS" --snapshot-kind invalid
expect_rejection "Worker snapshot requires product profile full" \
    bash "$FRESHNESS" --snapshot-kind worker --product-profile slim
worker_arches="$(bash -c 'source "$1"; snapshot_default_arches worker aarch64' _ "$FINGERPRINT")"
[[ "$worker_arches" == $'aarch64\nx86_64' ]] \
    || fail "a partial Worker snapshot set must require both Android ABIs"
host_arches="$(bash -c 'source "$1"; snapshot_default_arches host x86_64' _ "$FINGERPRINT")"
[[ "$host_arches" == $'aarch64\nx86_64' ]] \
    || fail "a partial host snapshot set must require both Android ABIs"
empty_worker_arch_count="$(bash -c 'source "$1"; mapfile -t arches < <(snapshot_default_arches worker); printf "%s" "${#arches[@]}"' _ "$FINGERPRINT")"
[[ "$empty_worker_arch_count" == "0" ]] \
    || fail "an absent default-off Worker snapshot set must remain optional"

echo "[2/5] checking snapshot-safe Worker bootstrap"
require_literal "$WORKER_JS" "__migoStartWorkerMessagePump" \
    "Worker JS must install a deferred private pump hook"
python3 - "$WORKER_JS" "$WORKER_RS" <<'PY'
import pathlib
import re
import sys

js = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
rust = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")

if re.search(r"(?m)^\s*_startMessagePump\(\);\s*$", js):
    raise SystemExit("Worker extension evaluation must not start the message pump")
if re.search(r"(?m)^\s*delete globalThis\.(?:Deno|__bootstrap);\s*$", js):
    raise SystemExit("Worker extension evaluation must retain deno_core restore globals")

hook_call = rust.find("__migoStartWorkerMessagePump")
publish = rust.find("*slot = Some(rt.v8_isolate().thread_safe_handle())")
module_load = rust.find("worker_load_module(&mut rt")
if min(hook_call, publish, module_load) < 0:
    raise SystemExit("Worker Rust bootstrap/handoff markers are incomplete")
if not hook_call < publish < module_load:
    raise SystemExit("Worker pump/hardening must precede isolate publication and game load")
PY

echo "[3/5] checking dedicated Cargo/runtime selection"
require_literal "$JS_RUNTIME/Cargo.toml" "worker-snapshot" \
    "js-runtime lacks the non-default worker-snapshot feature"
require_literal "$JS_RUNTIME/src/snapshot.rs" "WORKER_SNAPSHOT_BYTES" \
    "runtime lacks dedicated Worker snapshot bytes"
require_literal "$JS_RUNTIME/build.rs" "migo_has_worker_snapshot" \
    "build.rs lacks an independent Worker snapshot cfg"
require_literal "$ENGINE/tools/snapshot-gen/src/main.rs" "MIGO_SNAPSHOT_KIND" \
    "snapshot generator lacks kind selection"
require_literal "$WORKFLOW" 'git ls-files --error-unmatch "$bin" "$man"' \
    "snapshot workflow would mistake a new untracked Worker artifact for unchanged bytes"

echo "[4/5] checking isolated Android/AAR experiment routing"
for script in "$ANDROID_SO" "$ANDROID_SO_PS" "$AAR" "$AAR_PS"; do
    require_literal "$script" "worker-snapshot" \
        "$(basename "$script") lacks Worker snapshot selection"
done
require_literal "$GRADLE" "migoWorkerSnapshot" \
    "Gradle lacks the Worker snapshot selection property"
require_literal "$GRADLE" "migoWorkerSnapshotSuffix" \
    "Gradle does not isolate Worker snapshot JNI roots"
require_literal "$GRADLE" "workerSnapshotNeutralTasks" \
    "Gradle must classify non-build maintenance tasks explicitly"
require_literal "$GRADLE" "requestedTasks.any" \
    "Gradle must reject every non-full-release task, not only known build prefixes"
require_literal "$AAR" '"workerSnapshot": $WORKER_SNAPSHOT' \
    "Bash AAR metadata lacks Worker snapshot identity"
require_literal "$AAR_PS" 'workerSnapshot = $WorkerSnapshot.IsPresent' \
    "PowerShell AAR metadata lacks Worker snapshot identity"

# Exercise --skip-rust against a hermetic fixture containing only the baseline
# JNI root. The candidate must not borrow those libraries or reach Gradle.
mkdir -p "$TEST_TMP_ROOT/scripts" \
    "$TEST_TMP_ROOT/platforms/android/library" \
    "$TEST_TMP_ROOT/engine/jniLibs/full/arm64-v8a"
cp "$AAR" "$TEST_TMP_ROOT/scripts/build-aar.sh"
printf '#!/usr/bin/env bash\nexit 99\n' \
    > "$TEST_TMP_ROOT/scripts/build-android-so.sh"
chmod +x "$TEST_TMP_ROOT/scripts/build-android-so.sh"
: > "$TEST_TMP_ROOT/engine/jniLibs/full/arm64-v8a/libmigo.so"
: > "$TEST_TMP_ROOT/engine/jniLibs/full/arm64-v8a/libc++_shared.so"
expect_rejection \
    "jniLibs directory not found: $TEST_TMP_ROOT/engine/jniLibs/full-worker-snapshot" \
    bash "$TEST_TMP_ROOT/scripts/build-aar.sh" \
        --skip-rust --worker-snapshot release arm64-v8a

expect_rejection "Worker snapshot requires a full release build" \
    bash "$ANDROID_SO" debug --worker-snapshot
expect_rejection "Worker snapshot requires a full release build" \
    bash "$AAR" debug --worker-snapshot
expect_rejection "Worker snapshot requires a full release build" \
    bash "$ANDROID_SO" --product-profile slim --worker-snapshot
expect_rejection "Worker snapshot requires a full release build" \
    bash "$AAR" --product-profile slim --worker-snapshot

require_literal "$GENERATOR" '--locked' \
    "device snapshot generation must use the committed Cargo.lock"
require_literal "$WORKFLOW" '--locked' \
    "CI snapshot generation must use the committed Cargo.lock"

echo "[5/5] checking shell syntax"
bash -n "$FINGERPRINT" "$WRITE_MANIFEST" "$FRESHNESS" "$GENERATOR" \
    "$ANDROID_SO" "$AAR" "$0"

echo "PASS: R9 Worker snapshot experiment is kind-bound and isolated"
