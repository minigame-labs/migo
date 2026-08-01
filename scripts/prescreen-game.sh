#!/usr/bin/env bash
# What API surface does a mini-game bundle need, and does this build have it?
#
# The question a platform asks before migrating a catalogue is "how many of my
# games break". This answers the part that can be answered without running
# anything: which `wx.*` / `migo.*` names the bundle references, and which of
# those this build actually publishes.
#
# WHAT THIS DOES NOT TELL YOU -- read before quoting it at a customer:
#
#   * A name being published is not a working call. `wx.login` exists on every
#     build; whether it succeeds depends on the host installing an auth handler.
#     This reports the surface, never the behaviour.
#   * Static analysis under-reports by nature. Bundles reach APIs in ways no
#     scanner can resolve -- computed keys, aliases through data structures,
#     `eval`. Those are reported in their own bucket rather than folded into
#     "supported", because a scanner that quietly under-reports produces a
#     confident "everything is supported" and that is the one wrong answer that
#     costs a customer.
#   * Migration also fails on things that are not API names at all: DOM/CSS
#     assumptions, WebGL2, subpackage layout, timing. Running the game is the
#     only way to see those, and it has to be run on Android -- the Linux player
#     has no device services, so it reports failures Android would not have.
#
# Usage:
#   bash scripts/prescreen-game.sh <bundle-dir> [--surface FILE] [--out FILE]
#
#   --surface FILE  reuse a dump from scripts/dump-api-surface.sh instead of
#                   producing one (the dump costs a player run)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE=""
SURFACE=""
OUT=""
# Defaults to the local wx dump, which is git-ignored: absent for anyone who has
# not produced one, so the report says so rather than quietly merging buckets.
WXREF="$ROOT_DIR/tools/wx-api-diff/wx-android.json"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --surface) SURFACE="${2:?--surface requires a path}"; shift 2 ;;
        --surface=*) SURFACE="${1#*=}"; shift ;;
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        --wx-reference) WXREF="${2:?--wx-reference requires a path}"; shift 2 ;;
        --wx-reference=*) WXREF="${1#*=}"; shift ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) BUNDLE="$1"; shift ;;
    esac
done

[[ -n "$BUNDLE" ]] || { echo "usage: $0 <bundle-dir> [--surface FILE] [--out FILE]" >&2; exit 2; }
[[ -d "$BUNDLE" ]] || { echo "not a directory: $BUNDLE" >&2; exit 2; }

if [[ -z "$SURFACE" ]]; then
    SURFACE="$(mktemp)"
    trap 'rm -f "$SURFACE"' EXIT
    echo "[prescreen] dumping this build's published surface..." >&2
    bash "$ROOT_DIR/scripts/dump-api-surface.sh" --out "$SURFACE" >/dev/null
fi

python3 - "$ROOT_DIR" "$BUNDLE" "$SURFACE" "$WXREF" <<'PY' > "${OUT:-/dev/stdout}"
from __future__ import annotations

import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()
bundle = pathlib.Path(sys.argv[2]).resolve()
surface_path = pathlib.Path(sys.argv[3])

surface = json.loads(surface_path.read_text(encoding="utf-8"))
published_wx = set(surface.get("wx") or [])
published_migo = set(surface.get("migo") or [])
if not published_wx:
    print("ERROR: the surface dump has no `wx` names; refusing to report", file=sys.stderr)
    raise SystemExit(1)

# The wx reference table separates "this build lacks it" from "wx lacks it
# too", which are different findings. It is git-ignored, so it is absent on a
# fresh checkout -- and a report that silently merged the two buckets would
# inflate the gap count without saying why.
reference_path = pathlib.Path(sys.argv[4])
reference_wx: set[str] = set()
if reference_path.exists():
    reference_wx = set(json.loads(reference_path.read_text(encoding="utf-8")).get("wx", {}).keys())

sources = sorted(p for p in bundle.rglob("*.js") if p.is_file())
if not sources:
    print(f"ERROR: no .js under {bundle}; nothing to scan", file=sys.stderr)
    raise SystemExit(1)

IDENT = r"[A-Za-z_$][A-Za-z0-9_$]*"

# Names a bundle can bind to the namespace object. Once `const a = wx` appears,
# `a.foo` is a reference to `wx.foo`; missing that is the difference between a
# real answer and a confident wrong one on any minified bundle.
ALIAS = re.compile(rf"(?:const|let|var)\s+({IDENT})\s*=\s*(?:globalThis\s*\.\s*)?(wx|migo)\b")

DESTRUCTURE = re.compile(
    rf"(?:const|let|var)\s*\{{(?P<body>[^}}]*)\}}\s*=\s*(?:globalThis\s*\.\s*)?(?P<ns>wx|migo)\b"
)

# Access this scanner cannot resolve. Reported, never guessed at.
OPAQUE_PATTERNS = [
    (re.compile(rf"\b(?:wx|migo)\s*\[\s*(?!['\"])"), "computed member access -- `ns[expr]`"),
    (re.compile(r"\bObject\s*\.\s*(?:keys|getOwnPropertyNames|entries)\s*\(\s*(?:wx|migo)\b"),
     "reflection over the namespace"),
    (re.compile(r"\bfor\s*\(\s*(?:const|let|var)?\s*[A-Za-z_$][A-Za-z0-9_$]*\s+in\s+(?:wx|migo)\b"),
     "for-in over the namespace"),
    (re.compile(r"\beval\s*\("), "eval"),
]


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


referenced: dict[str, set[str]] = {"wx": set(), "migo": set()}
opaque: list[tuple[str, int, str]] = []
alias_names: dict[str, str] = {}

texts: dict[pathlib.Path, str] = {}
for path in sources:
    try:
        text = strip_comments(path.read_text(encoding="utf-8", errors="replace"))
    except OSError as error:
        print(f"ERROR: cannot read {path}: {error}", file=sys.stderr)
        raise SystemExit(1)
    texts[path] = text
    for match in ALIAS.finditer(text):
        alias_names[match.group(1)] = match.group(2)

for path, text in texts.items():
    rel = path.relative_to(bundle)
    for ns in ("wx", "migo"):
        for match in re.finditer(rf"\b{ns}\s*\.\s*({IDENT})", text):
            referenced[ns].add(match.group(1))
        for match in re.finditer(rf"\b{ns}\s*\[\s*['\"]([^'\"]+)['\"]\s*\]", text):
            referenced[ns].add(match.group(1))
    for match in DESTRUCTURE.finditer(text):
        ns = match.group("ns")
        for part in match.group("body").split(","):
            name = part.split(":")[0].strip()
            if re.fullmatch(IDENT, name):
                referenced[ns].add(name)
    for alias, ns in alias_names.items():
        for match in re.finditer(rf"\b{re.escape(alias)}\s*\.\s*({IDENT})", text):
            referenced[ns].add(match.group(1))
    for pattern, reason in OPAQUE_PATTERNS:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            opaque.append((str(rel), line, reason))

wx_used = referenced["wx"]
migo_used = referenced["migo"]
wx_missing = sorted(wx_used - published_wx)
migo_missing = sorted(migo_used - published_migo)
wx_ok = sorted(wx_used & published_wx)

# A name this build lacks reads very differently depending on whether wx has it:
# one is a gap in this runtime, the other is content calling something that does
# not exist on wx either (a typo, an adapter shim, a different platform's API).
missing_in_wx_too = [n for n in wx_missing if reference_wx and n not in reference_wx]
missing_but_wx_has = [n for n in wx_missing if not reference_wx or n in reference_wx]

out = []
out.append(f"# Prescreen: `{bundle.name}`")
out.append("")
out.append(f"- bundle: `{bundle}`")
out.append(f"- scanned: {len(sources)} JavaScript file(s)")
out.append(f"- this build publishes: {len(published_wx)} `wx` names, {len(published_migo)} `migo` names")
out.append("")
out.append("## Summary")
out.append("")
out.append("| | count |")
out.append("|---|---:|")
out.append(f"| `wx.*` names referenced | {len(wx_used)} |")
out.append(f"| of those, published by this build | {len(wx_ok)} |")
if reference_wx:
    out.append(f"| **not published, and wx has it** | **{len(missing_but_wx_has)}** |")
    out.append(f"| not published, and not a wx API either | {len(missing_in_wx_too)} |")
else:
    out.append(f"| **not published by this build** | **{len(missing_but_wx_has)}** |")
    out.append("| of those, how many wx has | *unknown -- no reference table* |")
out.append(f"| `migo.*` names not published | {len(migo_missing)} |")
out.append(f"| **sites this scanner cannot resolve** | **{len(opaque)}** |")
out.append("")

if not reference_wx:
    out.append("## wx reference table unavailable")
    out.append("")
    out.append(
        f"`{reference_path}` was not found, so this report **cannot separate** "
        "\"this build lacks it\" from \"wx lacks it too\". Every unpublished name "
        "below is listed as a gap, which overstates them: some are likely adapter "
        "shims or other platforms' SDKs that wx never had either."
    )
    out.append("")
    out.append(
        "Produce one with `tools/wx-api-diff/wx-api-dump.js` against a real wx "
        "runtime, or pass `--wx-reference FILE`."
    )
    out.append("")

if missing_but_wx_has:
    header = (
        "## Referenced, wx has it, this build does not"
        if reference_wx
        else "## Referenced but not published by this build"
    )
    out.append(header)
    out.append("")
    out.append(
        "Each of these is a real gap for this bundle."
        if reference_wx
        else "Unclassified -- see the note above about the missing reference table."
    )
    out.append("")
    for name in missing_but_wx_has:
        out.append(f"- `wx.{name}`")
    out.append("")

if missing_in_wx_too:
    out.append("## Referenced but not a wx API either")
    out.append("")
    out.append(
        "Usually an adapter shim, another platform's SDK, or dead code. Worth a "
        "look, but not evidence of a gap in this runtime."
    )
    out.append("")
    for name in missing_in_wx_too:
        out.append(f"- `{name}`")
    out.append("")

if migo_missing:
    out.append("## `migo.*` names not published")
    out.append("")
    for name in migo_missing:
        out.append(f"- `migo.{name}`")
    out.append("")

out.append("## Sites this scanner cannot resolve")
out.append("")
if not opaque:
    out.append(
        "None. Every namespace access in this bundle is a literal name, so the "
        "counts above cover what it references."
    )
else:
    out.append(
        f"{len(opaque)} site(s). The bundle reaches the namespace in ways static "
        "analysis cannot follow, so **the counts above are a lower bound** -- it "
        "may use names that do not appear here."
    )
    out.append("")
    seen = set()
    for rel, line, reason in opaque[:40]:
        key = (rel, reason)
        if key in seen:
            continue
        seen.add(key)
        out.append(f"- `{rel}:{line}` — {reason}")
    if len(opaque) > 40:
        out.append(f"- ... and {len(opaque) - 40} more")
out.append("")

out.append("## What this report is not")
out.append("")
out.append(
    "- **Not a compatibility verdict.** A published name is not a working call: "
    "`wx.login` exists on every build and still fails without a host auth handler."
)
out.append(
    "- **Not a migration estimate.** DOM/CSS assumptions, WebGL2, subpackage "
    "layout and timing break games without any missing API name."
)
out.append(
    "- **Not a substitute for running it.** And it has to run on Android: the "
    "Linux player has no device services, so it reports failures Android would "
    "not have."
)

print("\n".join(out))
PY

if [[ -n "$OUT" ]]; then
    echo "[prescreen] report -> $OUT" >&2
fi
