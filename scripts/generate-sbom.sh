#!/usr/bin/env bash
# Produce a CycloneDX SBOM for the Rust dependency tree.
#
# Enterprise procurement asks two questions of a binary they are about to embed:
# what is in it (for vulnerability scanning) and under what licences (for legal
# review). CycloneDX is what their tooling reads, so the answer is generated
# rather than written by hand -- a hand-maintained inventory of 400-odd crates
# is wrong the day after it is written.
#
# Components come from `cargo metadata --locked`: the resolved graph, matching
# `Cargo.lock` exactly, so the SBOM describes the tree that was actually built
# rather than what the manifests permit.
#
# Packages whose licence cannot be determined are listed explicitly under a
# `licence-unknown` property rather than silently omitted or defaulted. An SBOM
# that quietly reports fewer obligations than exist is worse than one that says
# it does not know: the first is reviewed and passed, the second is reviewed and
# asked about.
#
# Usage:
#   bash scripts/generate-sbom.sh [--out FILE]
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT_DIR/dist/migo-sbom.cdx.json"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$(dirname "$OUT")"

meta="$(mktemp)"
trap 'rm -f "$meta"' EXIT

if ! (cd "$ROOT_DIR/engine" && cargo metadata --format-version 1 --locked) >"$meta" 2>/dev/null; then
    echo "ERROR: cargo metadata failed; the SBOM would describe an unresolved tree" >&2
    exit 1
fi

python3 - "$ROOT_DIR" "$meta" "$OUT" <<'PY'
from __future__ import annotations

import datetime
import json
import os
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1]).resolve()
metadata = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
out_path = pathlib.Path(sys.argv[3])

packages = metadata.get("packages", [])
if not packages:
    print("ERROR: cargo metadata reported no packages", file=sys.stderr)
    raise SystemExit(1)

workspace_members = set(metadata.get("workspace_members", []))


def git(*args: str) -> str:
    try:
        return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()
    except Exception:
        return ""


commit = git("rev-parse", "HEAD")

components = []
unknown_licence = []

for package in sorted(packages, key=lambda p: (p["name"], p["version"])):
    # Workspace crates are the subject of the SBOM, not dependencies of it.
    if package.get("id") in workspace_members:
        continue

    name = package["name"]
    version = package["version"]
    licence = package.get("license")

    component = {
        "type": "library",
        "name": name,
        "version": version,
        "purl": f"pkg:cargo/{name}@{version}",
        "scope": "required",
    }

    if licence:
        # Kept as the raw SPDX expression rather than split: `MIT OR Apache-2.0`
        # is a choice the consumer makes, and splitting it into two licences
        # would report both as obligations.
        component["licenses"] = [{"expression": licence}]
    else:
        unknown_licence.append(f"{name}@{version}")
        component["properties"] = [
            {
                "name": "migo:licence-unknown",
                "value": "no license field in the crate manifest; "
                         "check the crate's LICENSE files before shipping",
            }
        ]

    if package.get("description"):
        component["description"] = package["description"][:400]
    if package.get("repository"):
        component["externalReferences"] = [
            {"type": "vcs", "url": package["repository"]}
        ]

    components.append(component)

# `SOURCE_DATE_EPOCH` if set, else now, always UTC. An SBOM is a release artifact
# (`release.yml` writes it to the Android dist directory), so a wall clock in it
# means two builds of one commit produce different bytes -- the property Phase 1's
# same-source rebuild comparison is built on. Refuses a malformed value rather than
# falling back, because a caller that set it believes it is producing something
# reproducible.
def sbom_timestamp():
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch is None or epoch == "":
        when = datetime.datetime.now(datetime.timezone.utc)
    else:
        if not epoch.isdigit():
            raise SystemExit(
                f"SOURCE_DATE_EPOCH must be non-negative Unix seconds, got: {epoch}"
            )
        when = datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc)
    return when.replace(microsecond=0).isoformat().replace("+00:00", "Z")


sbom = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.5",
    "version": 1,
    "metadata": {
        "timestamp": sbom_timestamp(),
        "component": {
            "type": "library",
            "name": "migo",
            "version": commit or "unknown",
            "description": "Embeddable Canvas/WebGL mini-game runtime",
        },
        "properties": [
            {"name": "migo:commit", "value": commit or "unknown"},
            {"name": "migo:components", "value": str(len(components))},
            {"name": "migo:licence-unknown-count", "value": str(len(unknown_licence))},
        ],
    },
    "components": components,
}

out_path.write_text(json.dumps(sbom, indent=2, sort_keys=False) + "\n", encoding="utf-8")

print(f"SBOM -> {out_path}")
print(f"  components: {len(components)}")
if unknown_licence:
    # Surfaced on stderr as well: a build log that scrolls past this is how an
    # unreviewed obligation ends up in a shipped artefact.
    print(
        f"  WARNING: {len(unknown_licence)} component(s) declare no licence: "
        + ", ".join(unknown_licence),
        file=sys.stderr,
    )
PY
