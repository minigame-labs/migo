#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])


def load(relative):
    return json.loads((root / relative).read_text(encoding="utf-8"))


project = load("platforms/openharmony/oh-package.json5")
entry = load("platforms/openharmony/entry/oh-package.json5")
application = load("platforms/openharmony/AppScope/app.json5")["app"]

errors = []
placeholder = "Please " + "describe"
if placeholder.lower() in str(project.get("description", "")).lower():
    errors.append("project description is still the generated placeholder")
if placeholder.lower() in str(entry.get("description", "")).lower():
    errors.append("entry description is still the generated placeholder")
if not str(entry.get("author", "")).strip():
    errors.append("entry author is empty")
if entry.get("license") != "BSL-1.1":
    errors.append("entry license must identify the repository license")
if application.get("vendor") in {None, "", "example"}:
    errors.append("application vendor is a placeholder")
if application.get("bundleName") != "com.migo.ohoshost":
    errors.append("application bundle name drifted")
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(application.get("versionName", ""))):
    errors.append("application versionName is not a three-part version")
if not isinstance(application.get("versionCode"), int) or application["versionCode"] <= 0:
    errors.append("application versionCode must be a positive integer")

if errors:
    for error in errors:
        print(f"OpenHarmony package metadata contract failed: {error}", file=sys.stderr)
    raise SystemExit(1)

print("OpenHarmony package metadata contract: PASS")
PY
