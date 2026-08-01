#!/usr/bin/env bash
# Every op that reaches a gated capability must ask permission first.
#
# A permission system that covers most of its surface is not most of a
# permission system. It advertises a guarantee it does not hold, to a customer
# who is making a compliance decision on the strength of it -- the same defect
# shape as an ad bridge that lets content mint its own rewards, and the reason
# this gate exists before the enforcement it checks.
#
# The required set is *derived*, not registered. An op is subject to a check
# because it reaches for a gated accessor, so this reads `GATED_ACCESSORS` from
# `shared/src/services/permission.rs` and then finds every function in the
# runtime that calls one. Add an op that uses the camera tomorrow and the gate
# demands its guard without anybody remembering to list it -- which is exactly
# what a hand-maintained op register does not do, and how surfaces like this
# drift out from under their own tests.
#
# Both directions are checked:
#   forward  -- a function using a gated accessor must call `require_scope`;
#   backward -- with the scope that accessor maps to, not merely some scope.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()

mapping_rs = root / "engine/crates/shared/src/services/permission.rs"
helper_rs = root / "engine/crates/runtime-v8/src/permission.rs"
runtime_src = root / "engine/crates/runtime-v8/src"

for required in (mapping_rs, helper_rs, runtime_src):
    if not required.exists():
        print(f"ERROR: {required} not found; this gate cannot check anything", file=sys.stderr)
        sys.exit(1)


def strip_comments(text: str) -> str:
    """Commented-out code is not an implementation.

    Applied before every match below, because a `// require_scope(...)` reads as
    protection to a grep and as nothing at all to a compiler.
    """
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


failures: list[str] = []

# --- The mapping, read from its declaration -------------------------------
mapping_source = strip_comments(mapping_rs.read_text(encoding="utf-8"))
block = re.search(
    r"GATED_ACCESSORS\s*:\s*&\[\(&str,\s*Scope\)\]\s*=\s*&\[(?P<body>.*?)\];",
    mapping_source,
    re.DOTALL,
)
if not block:
    print(
        f"ERROR: no `GATED_ACCESSORS` table in {mapping_rs.relative_to(root)}; "
        "the set of gated capabilities is the input to this gate and it is gone",
        file=sys.stderr,
    )
    sys.exit(1)

gated = dict(re.findall(r'\(\s*"([a-z_]+)"\s*,\s*Scope::([A-Za-z]+)\s*\)', block.group("body")))
if not gated:
    print(
        f"ERROR: `GATED_ACCESSORS` is empty in {mapping_rs.relative_to(root)}; "
        "the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

# --- The helper has to exist and refuse by default ------------------------
helper_source = strip_comments(helper_rs.read_text(encoding="utf-8"))
if not re.search(r"\bfn\s+require_scope\s*\(", helper_source):
    failures.append(
        f"{helper_rs.relative_to(root)}: `require_scope` is gone; every call site "
        "below would be calling nothing"
    )
if "unwrap_or(false)" not in helper_source and "ScopeState::Granted" not in helper_source:
    failures.append(
        f"{helper_rs.relative_to(root)}: `require_scope` no longer decides on "
        "`ScopeState::Granted`; it may no longer deny by default"
    )

# --- Every function using a gated accessor must guard ---------------------
#
# Functions are split on top-level `fn` boundaries. Nested functions would blur
# the boundary, so a nested `fn` inside a guarded one is treated as part of it:
# that is the conservative direction, and the ops in question are flat.
FUNCTION = re.compile(r"^(?:pub(?:\([a-z()]+\))?\s+)?(?:async\s+)?fn\s+([a-z_0-9]+)", re.M)

checked_sites = 0
for source_path in sorted(runtime_src.rglob("*.rs")):
    if "/tests/" in str(source_path) or source_path.name == "permission.rs":
        continue
    text = strip_comments(source_path.read_text(encoding="utf-8", errors="replace"))

    starts = [(m.start(), m.group(1)) for m in FUNCTION.finditer(text)]
    if not starts:
        continue
    bounds = [
        (name, start, starts[i + 1][0] if i + 1 < len(starts) else len(text))
        for i, (start, name) in enumerate(starts)
    ]

    for name, start, end in bounds:
        body = text[start:end]
        for accessor, scope in gated.items():
            if not re.search(rf"\.\s*{accessor}\s*\(\s*\)", body):
                continue
            checked_sites += 1
            expected = rf"require_scope\s*\([^)]*Scope::{scope}\s*\)"
            if not re.search(expected, body):
                rel = source_path.relative_to(root)
                line = text.count("\n", 0, start) + 1
                failures.append(
                    f"{rel}:{line}: `{name}` reaches `services.{accessor}()` without "
                    f"`require_scope(.., Scope::{scope})`. A capability the host "
                    "never granted is reachable through it."
                )

if checked_sites == 0:
    failures.append(
        "no call site of any gated accessor was found anywhere in the runtime. "
        "Either the accessors were renamed or this gate stopped matching them; "
        "either way it is now checking nothing."
    )

if failures:
    print("FAIL: permission coverage contract", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)
    sys.exit(1)

print(
    "PASS: permission coverage contract "
    f"({len(gated)} gated accessor(s), {checked_sites} guarded call site(s))"
)
PY
