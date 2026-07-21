#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INTEGRITY="$ROOT/engine/crates/shared/src/vfs/integrity.rs"
GAME_PATHS="$ROOT/engine/crates/shared/src/vfs/game_paths.rs"
IO_TASK="$ROOT/engine/crates/io/src/task.rs"
IO_SCHEDULER="$ROOT/engine/crates/io/src/scheduler.rs"
HOST_RUNTIME="$ROOT/engine/crates/runtime-v8/src/host_runtime.rs"

fail() {
  printf 'R8 install-receipt contract failure: %s\n' "$*" >&2
  exit 1
}

grep -Fq 'MAX_MANIFEST_BYTES' "$INTEGRITY" || fail 'manifest read is not bounded'
grep -Fq 'MAX_RECEIPT_BYTES' "$INTEGRITY" || fail 'receipt read is not bounded'
grep -Fq 'PROMOTION_HASH_BUFFER_BYTES: usize = 64 * 1024' "$INTEGRITY" || fail 'promotion does not reuse a 64-KiB hash buffer'
grep -Fq 'MAX_CODE_TREE_ENTRIES' "$INTEGRITY" || fail 'physical tree traversal is not bounded'
grep -Fq 'verify_launch_receipt' "$INTEGRITY" || fail 'cheap receipt verification API is absent'
grep -Fq 'verify_and_promote_for_launch' "$INTEGRITY" || fail 'full promotion API is absent'
grep -Fq 'flock' "$INTEGRITY" || fail 'concurrent promotion lacks a process-safe lock'
grep -Fq 'O_NONBLOCK' "$INTEGRITY" || fail 'host integrity probes can block while opening a FIFO'
grep -Fq 'integrity lock must be a regular file' "$INTEGRITY" || fail 'promotion lock accepts a special file'
grep -Fq 'sync_all()' "$INTEGRITY" || fail 'receipt commit is not durability-synchronized'
grep -Fq 'integrity_receipt_path' "$GAME_PATHS" || fail 'receipt is not stored beside persistent code'
grep -Fq 'VerifyPackage' "$IO_TASK" || fail 'package verification has no explicit scheduler request'
grep -Fq 'IoRequest::VerifyPackage { .. } => RouteDecision::Delegated(PoolKind::Fs)' "$IO_SCHEDULER" || fail 'package verification is not routed to the bounded FS pool'
grep -Fq 'run_package_verification' "$IO_SCHEDULER" || fail 'same-package verification is not coalesced before worker dispatch'
grep -Fq 'verify_launch_receipt' "$HOST_RUNTIME" || fail 'launch does not attempt the cheap receipt path'
grep -Fq '.run_package_verification(' "$HOST_RUNTIME" || fail 'receipt miss bypasses keyed async verification'

eval_start="$(grep -n 'pub async fn evaluate_module' "$HOST_RUNTIME" | head -1 | cut -d: -f1)"
eval_end="$(awk -v start="$eval_start" 'NR > start && /^    pub / { print NR; exit }' "$HOST_RUNTIME")"
if [[ -z "$eval_start" || -z "$eval_end" ]]; then
  fail 'cannot isolate HostJsRuntime::evaluate_module'
fi
if sed -n "${eval_start},${eval_end}p" "$HOST_RUNTIME" | grep -Fq '.verify_all_files(';
then
  fail 'evaluate_module still unconditionally hashes the full package'
fi

printf 'R8 persistent install-receipt static contract: PASS\n'
