#!/usr/bin/env bash
# A command that waits for an answer is a decision, so it has to be written down.
#
# Two things rest on the size of this set, and neither survives an impression of
# it.
#
# On Apple's Performance+ lane, content runs in an agent inside WebKit's
# WebContent process. The choice between a Dedicated Worker and the Window agent
# is not a performance trade: the Window main agent's `[[CanBlock]]` is false, so
# `Atomics.wait` throws there, and a synchronous reply is not slower -- it does
# not exist. The size of this set is therefore the size of what the Window lane
# would have to give up, and `docs/apple-final-implementation-plan.md` E1 named
# four examples where the derived answer is twenty-three content-visible ones.
#
# On Android, today, each of these stalls the JavaScript thread for a round trip.
# Two of them -- `ClientWaitSync` and `GetQueryParameter` -- are designed to be
# polled every frame.
#
# THE DRIFT THIS EXISTS TO CATCH is a new one appearing without anyone deciding
# to add it. Nothing else in the build reports it: a variant gains a `resp`
# field, the caller waits, and the only symptom is a frame that took longer on a
# device nobody was holding.
#
# The list is DERIVED -- the protocol enums are parsed for variants carrying a
# reply channel -- and the contract supplies only the classification, which
# cannot be. A command in the source and not the contract fails, and so does one
# in the contract and not the source.
#
# Host-only: reads two files.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

CONTRACT = Path("contracts/runtime/synchronous-surface.json")
ENUMS = ["GLCmd", "Canvas2DCmd", "RenderCommand", "CanvasCmd"]

ANSWERS = {"yes", "partly", "no", "n_a"}
REPLIES = {"always", "optional"}

problems = []

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
sources = [Path(s) for s in contract["sources"]]
for path in sources:
    if not path.is_file():
        print(f"FAIL: {path} is missing; the surface cannot be derived.", file=sys.stderr)
        raise SystemExit(1)


def strip_line_comments(text):
    """Drop `//` comments so a `resp` named in prose is not read as a field."""
    out, index, end = [], 0, len(text)
    while index < end:
        if text.startswith("//", index):
            newline = text.find("\n", index)
            index = end if newline < 0 else newline
        else:
            out.append(text[index])
            index += 1
    return "".join(out)


def enum_body(text, name):
    match = re.search(r"\benum\s+%s\s*\{" % name, text)
    if not match:
        return None
    index, depth = match.end(), 1
    while depth:
        depth += (text[index] == "{") - (text[index] == "}")
        index += 1
    return text[match.end() : index - 1]


def variants(body):
    found, depth, start = [], 0, 0
    for index, character in enumerate(body):
        if character in "{(<[":
            depth += 1
        elif character in "})>]":
            depth -= 1
        elif character == "," and depth == 0:
            found.append(body[start:index])
            start = index + 1
    found.append(body[start:])
    return [variant.strip() for variant in found if variant.strip()]


def reply_kind(variant):
    """`always`, `optional`, or None when the variant carries no reply channel."""
    if "{" not in variant:
        return None
    fields = variant[variant.find("{") + 1 : variant.rfind("}")]
    depth, start = 0, 0
    pieces = []
    for index, character in enumerate(fields):
        if character in "{(<[":
            depth += 1
        elif character in "})>]":
            depth -= 1
        elif character == "," and depth == 0:
            pieces.append(fields[start:index])
            start = index + 1
    pieces.append(fields[start:])
    for piece in pieces:
        if not re.match(r"\s*resp\s*:", piece):
            continue
        # `Option<RenderCmdResp<T>>` is a channel that may be absent;
        # `RenderCmdResp<Option<T>>` is a channel that always exists and
        # carries a value that may be absent. Only the outermost type says
        # whether the caller waits, so this reads the head of the type rather
        # than searching it.
        kind = piece.split(":", 1)[1].strip()
        return "optional" if kind.startswith("Option<") else "always"
    return None


derived = {}
for path in sources:
    text = strip_line_comments(path.read_text(encoding="utf-8"))
    for enum in ENUMS:
        body = enum_body(text, enum)
        if body is None:
            continue
        seen_any = False
        for variant in variants(body):
            name = re.match(r"(?:#\[[^\]]*\]\s*)*([A-Z][A-Za-z0-9_]*)", variant)
            if not name:
                continue
            seen_any = True
            kind = reply_kind(variant)
            if kind:
                derived[f"{enum}::{name.group(1)}"] = kind
        if not seen_any:
            problems.append(f"no variants parsed out of {enum}; the pattern no longer matches")

if len(derived) < 20:
    problems.append(
        f"only {len(derived)} reply-carrying commands derived; the scan no longer matches"
    )

classified = contract["commands"]
missing = sorted(set(derived) - set(classified))
extra = sorted(set(classified) - set(derived))

for name in missing:
    problems.append(
        f"{name} waits for a reply and is not in the contract. "
        "Adding one is a decision: on Apple it is something the Window agent "
        "cannot do at all, and on Android it is a per-frame stall."
    )
for name in extra:
    problems.append(f"{name} is in the contract and no longer waits for a reply; remove it")

for name, kind in sorted(derived.items()):
    entry = classified.get(name)
    if entry is None:
        continue
    if entry.get("reply") != kind:
        problems.append(
            f"{name}: the contract says its reply channel is {entry.get('reply')!r}, "
            f"the source says {kind!r}"
        )
    answer = entry.get("producer_answers_locally")
    if answer not in ANSWERS:
        problems.append(
            f"{name}: producer_answers_locally is {answer!r}, not one of {sorted(ANSWERS)}"
        )
    if entry.get("reply") not in REPLIES:
        problems.append(f"{name}: reply is {entry.get('reply')!r}, not one of {sorted(REPLIES)}")
    if not entry.get("note"):
        problems.append(f"{name}: has no note saying why it waits or why it need not")

print()
content = {
    name: entry
    for name, entry in classified.items()
    if entry.get("producer_answers_locally") != "n_a"
}
crossing = [
    name for name, entry in content.items() if entry.get("producer_answers_locally") == "no"
]
partial = [
    name for name, entry in content.items() if entry.get("producer_answers_locally") == "partly"
]
print(f"  {len(derived)} commands carry a reply channel")
print(f"  {len(content)} of them are reachable from content")
print(f"    {len(crossing)} cross on every call")
print(f"    {len(partial)} answer some calls locally and cross the rest")
print(f"  {len(classified) - len(content)} are host lifecycle, not content")

if problems:
    print()
    print("FAIL: the synchronous surface and its contract disagree.", file=sys.stderr)
    for problem in problems:
        print(f"  * {problem}", file=sys.stderr)
    raise SystemExit(1)

print()
print("PASS: every command that waits for an answer is classified.")
PY
