#!/usr/bin/env bash
# Whatever `harden_global_scope` removes must never have been mirrored.
#
# The runtime publishes two namespaces to content, `wx` and `migo`, built in
# `97_migo_namespace.js` by copying property descriptors off globalThis during
# bootstrap. deno_core's internals are removed from globalThis afterwards, by
# `harden_global_scope` in Rust -- deleting them from JS instead breaks
# deno_core's snapshot restore path, so the ordering is forced.
#
# A mirror built first therefore captures whatever hardening deletes later, and
# deleting the global does nothing to the copy. That is how `wx.Deno.core.ops`
# ended up handing content 616 invocable ops, past every JS-level API and the
# policies built on them. `__bootstrap` escaped only because the mirror filter
# happens to skip underscore-prefixed names.
#
# So the two lists have to agree, and nothing keeps them agreeing on its own:
# the deletions live in Rust, the exclusions in JS, and adding to one reads as
# complete on its own. This gate reads both from source and fails if hardening
# removes something the mirrors would still publish.
#
# The behavioural backstop is `tests/published_namespace_isolation.rs`, which
# searches the published namespaces for an op table by shape rather than by
# name. Both matter: this gate catches the drift at the point it is introduced,
# the test catches a leak that arrives by some route nobody listed.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()

harden_rs = root / "engine/crates/runtime-v8/src/lib.rs"
namespace_js = root / "engine/crates/runtime-v8/src/97_migo_namespace.js"

for required in (harden_rs, namespace_js):
    if not required.exists():
        print(f"ERROR: {required} not found; this gate cannot check anything", file=sys.stderr)
        sys.exit(1)

failures: list[str] = []


def strip_comments(text: str) -> str:
    """Drop comments before matching.

    Commented-out code is not an implementation, but it is still text, so any
    check that greps source has to remove it first or it reads `// delete
    globalThis.Deno;` as protection. Applied to both files rather than to
    whichever one currently needs it, because the next reader will add a check
    to the other one.
    """
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


# --- What hardening removes -------------------------------------------------
#
# Read the deletions out of the script `harden_global_scope` executes, rather
# than out of a list kept here: a gate that carries its own copy of the answer
# stops tracking the thing it guards.
harden_source = strip_comments(harden_rs.read_text(encoding="utf-8"))
harden_fn = re.search(
    r"pub fn harden_global_scope\s*\([^)]*\)\s*\{(?P<body>.*?)\n\}",
    harden_source,
    re.DOTALL,
)
if not harden_fn:
    print(
        f"ERROR: could not find `harden_global_scope` in {harden_rs.relative_to(root)}; "
        "the hardening step moved or was renamed and this gate is now blind",
        file=sys.stderr,
    )
    sys.exit(1)

deleted = set(re.findall(r"delete\s+globalThis\.([A-Za-z_$][A-Za-z0-9_$]*)", harden_fn.group("body")))

# --- What the mirrors refuse to publish -------------------------------------
namespace_source = strip_comments(namespace_js.read_text(encoding="utf-8"))
internals_block = re.search(
    r"const\s+_RUNTIME_INTERNALS\s*=\s*new\s+Set\(\[(?P<body>.*?)\]\)",
    namespace_source,
    re.DOTALL,
)
if not internals_block:
    print(
        f"ERROR: no `_RUNTIME_INTERNALS` set in {namespace_js.relative_to(root)}; "
        "the mirror has no declared internals to exclude",
        file=sys.stderr,
    )
    sys.exit(1)

excluded = set(re.findall(r'"([^"]+)"', internals_block.group("body")))

# The set has to be spliced into the exclusion the mirror actually consults;
# declaring it and not using it would read as protection that is not there.
if not re.search(r"_NON_API\s*=\s*new\s+Set\(\[.*?\.\.\._RUNTIME_INTERNALS.*?\]\)", namespace_source, re.DOTALL):
    failures.append(
        f"{namespace_js.relative_to(root)}: `_RUNTIME_INTERNALS` is declared but not "
        "spread into `_NON_API`, so the mirror never consults it"
    )

# --- Anti-vacuity -----------------------------------------------------------
if not deleted:
    failures.append(
        f"{harden_rs.relative_to(root)}: `harden_global_scope` deletes nothing; "
        "either hardening was removed or this gate stopped matching it, and "
        "either way the comparison below proves nothing"
    )
if not excluded:
    failures.append(
        f"{namespace_js.relative_to(root)}: `_RUNTIME_INTERNALS` is empty; the "
        "gate would pass vacuously"
    )

# --- The contract -----------------------------------------------------------
for name in sorted(deleted - excluded):
    failures.append(
        f"`{name}` is deleted from globalThis by harden_global_scope but is not in "
        f"`_RUNTIME_INTERNALS` ({namespace_js.relative_to(root)}). The wx/migo "
        f"mirrors are built before hardening runs, so they captured `{name}` and "
        "deleting the global does not reach the copy -- content can still get it "
        "off `wx` or `migo`."
    )

if failures:
    print("FAIL: runtime internals must not be published to content", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)
    sys.exit(1)

print(
    "PASS: runtime internals not published "
    f"(hardening removes {sorted(deleted)}; mirrors exclude {sorted(excluded)})"
)
PY
