#!/usr/bin/env bash
# Does this bundle actually run? The half a scanner cannot answer.
#
# `scripts/prescreen-game.sh` reports which API names a bundle needs and whether
# this build publishes them. That is a real answer to a narrow question, and on
# its own it is the wrong shape to give a customer: migration also fails on
# things that are not API names -- DOM assumptions, WebGL2, subpackage layout,
# timing, a shader that compiles everywhere except here. None of those appear in
# a name diff. The only way to see them is to run the bundle.
#
# ON A DEVICE, NOT ON THE LINUX PLAYER. This is not a preference. The Linux
# player has no device services, so any content that touches login, storage or
# system info gets "not supported" and the report fills with blockers that do
# not exist on Android -- judging a migratable customer un-migratable, which is
# the exact inverse of what this tool is for.
#
# WHAT IT CAN AND CANNOT SEE:
#
#   * It observes the screen and the log. It does not instrument the content, so
#     it works on a bundle that was never built with this in mind -- which is the
#     whole point, since the bundle belongs to someone else.
#   * "Painted" here means the surface carries a real image: many distinct
#     colours rather than one flat fill. A game that legitimately renders a solid
#     colour for its first seconds reads as not-painted, and the report says the
#     evidence is a screenshot so a human can overrule it. A verdict this tool
#     states without evidence attached would be worse than no verdict.
#   * "Still painting" is two captures a few seconds apart differing. A finished
#     static scene reads as stalled. Same remedy: the PNGs are attached.
#   * A clean run is not a certification. It means nothing blocked in the first
#     N seconds on one device.
#
# Usage:
#   scripts/prescreen-run.sh <bundle-dir> --package PKG --activity NAME
#                            [--device SERIAL] [--game-id ID] [--secs N]
#                            [--out FILE] [--keep]
#
#   --package   an installed host that loads a bundle from its own private
#               directory. The report names it, because "it ran" means nothing
#               without saying what ran it.
#   --activity  the activity that puts the *game* on screen. Required, and not
#               guessed: the first version resolved the launcher activity, landed
#               on a host's menu screen, saw a colourful animated-enough surface
#               and reported "Runs" for a bundle that had never been loaded. A
#               host's own UI is indistinguishable from content by pixels alone,
#               so the caller has to say which window is the game, and this checks
#               that window is the one in front.
#   --game-id   which slot under <files>/migo/games/<id>/code the host reads
#               (default: demo)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BUNDLE=""; PKG=""; SERIAL="${ANDROID_SERIAL:-}"; ACTIVITY=""; GAME_ID="demo"
SECS=20; OUT=""; KEEP=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PKG="${2:?--package requires a value}"; shift 2 ;;
        --package=*) PKG="${1#*=}"; shift ;;
        --device) SERIAL="${2:?--device requires a value}"; shift 2 ;;
        --device=*) SERIAL="${1#*=}"; shift ;;
        --activity) ACTIVITY="${2:?--activity requires a value}"; shift 2 ;;
        --activity=*) ACTIVITY="${1#*=}"; shift ;;
        --game-id) GAME_ID="${2:?--game-id requires a value}"; shift 2 ;;
        --game-id=*) GAME_ID="${1#*=}"; shift ;;
        --secs) SECS="${2:?--secs requires a number}"; shift 2 ;;
        --secs=*) SECS="${1#*=}"; shift ;;
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        --keep) KEEP=true; shift ;;
        -h|--help) sed -n '2,45p' "$0"; exit 0 ;;
        *) BUNDLE="$1"; shift ;;
    esac
done

[[ -n "$BUNDLE" ]] || { echo "usage: $0 <bundle-dir> --package PKG" >&2; exit 2; }
[[ -d "$BUNDLE" ]] || { echo "not a directory: $BUNDLE" >&2; exit 2; }
[[ -n "$PKG" ]] || { echo "--package is required: this reports what ran the bundle, and an unnamed host is not evidence" >&2; exit 2; }
[[ -f "$BUNDLE/game.js" ]] || { echo "no game.js in $BUNDLE; the host loads game.js from the bundle root" >&2; exit 2; }

ADB_BIN="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
[[ -x "$ADB_BIN" ]] || ADB_BIN="$(command -v adb || true)"
[[ -n "$ADB_BIN" ]] || { echo "no adb; set ADB=/path/to/adb" >&2; exit 2; }
ADB=("$ADB_BIN")
[[ -n "$SERIAL" ]] && ADB+=(-s "$SERIAL")

# One device or an explicit serial. Guessing between two attached devices is how
# a report ends up describing hardware nobody asked about.
attached="$("${ADB[@]}" devices | awk 'NR>1 && $2=="device" {n++} END {print n+0}')"
if [[ -z "$SERIAL" && "$attached" -gt 1 ]]; then
    echo "$attached devices attached; pass --device SERIAL" >&2
    exit 2
fi
[[ "$attached" -ge 1 ]] || { echo "no device attached" >&2; exit 2; }

"${ADB[@]}" shell "pm path $PKG" >/dev/null 2>&1 || {
    echo "package not installed on the device: $PKG" >&2; exit 2; }

if [[ -z "$ACTIVITY" ]]; then
    echo "--activity is required: name the activity that puts the game on screen." >&2
    echo "  Resolving the launcher instead lands on a host's menu, where a colourful" >&2
    echo "  screen means the host is fine and says nothing about the bundle." >&2
    exit 2
else
    [[ "$ACTIVITY" == */* ]] || ACTIVITY="$PKG/$ACTIVITY"
fi

WORK="$(mktemp -d)"
cleanup() { [[ "$KEEP" == true ]] || rm -rf "$WORK"; }
trap cleanup EXIT

say() { echo "[prescreen-run] $*" >&2; }

DEVICE_MODEL="$("${ADB[@]}" shell getprop ro.product.model 2>/dev/null | tr -d '\r')"
DEVICE_SDK="$("${ADB[@]}" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')"

# ---- deploy -------------------------------------------------------------
# Through /data/local/tmp and `run-as`, so this needs no root and touches only
# the host's own sandbox. The bundle never leaves the customer's device.
say "deploying $(basename "$BUNDLE") to $PKG (slot: $GAME_ID)"
REMOTE_STAGE="/data/local/tmp/migo-prescreen-$$"
"${ADB[@]}" shell "rm -rf $REMOTE_STAGE && mkdir -p $REMOTE_STAGE" >/dev/null
tar -C "$BUNDLE" -cf "$WORK/bundle.tar" . 2>/dev/null
"${ADB[@]}" push "$WORK/bundle.tar" "$REMOTE_STAGE/bundle.tar" >/dev/null 2>&1 \
    || { echo "adb push failed" >&2; exit 1; }
CODE_DIR="files/migo/games/$GAME_ID/code"
if ! "${ADB[@]}" shell "run-as $PKG sh -c 'rm -rf $CODE_DIR && mkdir -p $CODE_DIR && cd $CODE_DIR && tar -xf $REMOTE_STAGE/bundle.tar'" 2>&1 | grep -qv .; then
    :
fi
deployed="$("${ADB[@]}" shell "run-as $PKG sh -c 'ls $CODE_DIR/game.js 2>/dev/null'" | tr -d '\r')"
"${ADB[@]}" shell "rm -rf $REMOTE_STAGE" >/dev/null 2>&1 || true
[[ -n "$deployed" ]] || {
    echo "deploy failed: game.js is not in the host's private directory." >&2
    echo "  \`run-as\` needs a debuggable build of $PKG. A release host cannot be driven this way." >&2
    exit 1; }

# ---- run ----------------------------------------------------------------
"${ADB[@]}" logcat -c >/dev/null 2>&1 || true
"${ADB[@]}" shell "am force-stop $PKG" >/dev/null 2>&1 || true
say "launching $ACTIVITY for ${SECS}s"
# MIGO_CAPI_LOG=info is the only outlet a content-side exception has on a pure
# native host; without it a JS throw is a black screen and a clean log.
# `am start` reports failure in its output, not always in its exit code, and
# swallowing both leaves the caller with "exit 1" and nothing to act on.
am_out="$("${ADB[@]}" shell "am start -n $ACTIVITY --es migoGameId $GAME_ID --es MIGO_CAPI_LOG info" 2>&1 | tr -d '\r')"
if grep -qiE "error|does not exist|not exported|permission denial" <<<"$am_out"; then
    echo "could not start $ACTIVITY:" >&2
    sed 's/^/  /' <<<"$am_out" >&2
    echo "  Activities this host exports:" >&2
    "${ADB[@]}" shell "cmd package query-activities --brief -a android.intent.action.MAIN 2>/dev/null | grep $PKG" 2>/dev/null | sed 's/^/    /' >&2 || true
    echo "  A host that cannot be started into its game surface from adb cannot be" >&2
    echo "  prescreened this way; ask its owner for an entry point that takes a game id." >&2
    exit 1
fi

# Which window is in front, across the spellings Android has used. `dumpsys
# window windows` carries mCurrentFocus on older releases and not on API 31,
# where the same line lives under plain `dumpsys window` -- a probe that knew
# only the first spelling came back empty on a modern phone and the report read
# that silence as "the host was never in front", condemning the bundle for the
# tool's own blind spot.
# Every other Migo host on the device writes the same tags. Reading the whole
# buffer meant another app's session lifecycle counted as evidence for this
# bundle -- on a phone with the bench shell installed, "a session reached
# RUNNING" was true and had nothing to do with what was being prescreened. So
# the dump is bound to this launch's pid.
sleep 2
TARGET_PID="$("${ADB[@]}" shell "pidof $PKG" 2>/dev/null | tr -d '\r' | awk '{print $1}')"
[[ -n "$TARGET_PID" ]] && say "host pid $TARGET_PID"

focus_now() {
    local out
    out="$("${ADB[@]}" shell dumpsys window 2>/dev/null \
        | grep -m1 -oE 'mCurrentFocus=Window\{[^}]*\}' | tr -d '\r')"
    if [[ -z "$out" ]]; then
        out="$("${ADB[@]}" shell dumpsys activity activities 2>/dev/null \
            | grep -m1 -oE 'ResumedActivity: ActivityRecord\{[^}]*\}' | tr -d '\r')"
    fi
    printf '%s' "$out"
}

sleep "$(( SECS / 2 ))"
"${ADB[@]}" exec-out screencap -p > "$WORK/first.png" 2>/dev/null || true
focus_now > "$WORK/focus1.txt" 2>/dev/null || true
sleep "$(( SECS - SECS / 2 ))"
"${ADB[@]}" exec-out screencap -p > "$WORK/second.png" 2>/dev/null || true
focus_now > "$WORK/focus2.txt" 2>/dev/null || true
if [[ -n "$TARGET_PID" ]]; then
    "${ADB[@]}" logcat -d --pid="$TARGET_PID" > "$WORK/logcat.txt" 2>/dev/null || true
    # --pid is unsupported on older platform-tools/devices; an empty capture there
    # would read as "the content logged nothing", which is a finding rather than a
    # gap in the tool.
    if [[ ! -s "$WORK/logcat.txt" ]]; then
        "${ADB[@]}" logcat -d > "$WORK/logcat.txt" 2>/dev/null || true
        echo "unfiltered" > "$WORK/log-scope.txt"
    else
        echo "pid $TARGET_PID" > "$WORK/log-scope.txt"
    fi
else
    "${ADB[@]}" logcat -d > "$WORK/logcat.txt" 2>/dev/null || true
    echo "unfiltered (host pid not found)" > "$WORK/log-scope.txt"
fi

alive="$("${ADB[@]}" shell "pidof $PKG" 2>/dev/null | tr -d '\r')"
"${ADB[@]}" shell "am force-stop $PKG" >/dev/null 2>&1 || true

REPORT_DIR="${OUT:+$(dirname "$OUT")}"
REPORT_DIR="${REPORT_DIR:-$PWD}"
mkdir -p "$REPORT_DIR"
cp "$WORK/first.png" "$REPORT_DIR/prescreen-frame-1.png" 2>/dev/null || true
cp "$WORK/second.png" "$REPORT_DIR/prescreen-frame-2.png" 2>/dev/null || true

python3 - "$BUNDLE" "$WORK" "$PKG" "$ACTIVITY" "$DEVICE_MODEL" "$DEVICE_SDK" \
         "$SECS" "$alive" "$REPORT_DIR" <<'PY' > "${OUT:-/dev/stdout}"
from __future__ import annotations

import pathlib
import re
import sys
import zlib

bundle, work, pkg, activity, model, sdk, secs, alive, report_dir = sys.argv[1:10]
work = pathlib.Path(work)


def png_stats(path: pathlib.Path):
    """Distinct colours and a cheap content hash, without an image library.

    A prescreen has to run on a customer's machine; requiring Pillow to answer
    "is anything on screen" would be a dependency for a yes/no. This decodes the
    IDAT stream and samples it, which needs only zlib from the standard library.
    """
    if not path.exists() or path.stat().st_size == 0:
        return None
    raw = path.read_bytes()
    if raw[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    pos, idat = 8, bytearray()
    width = height = 0
    while pos + 8 <= len(raw):
        length = int.from_bytes(raw[pos : pos + 4], "big")
        kind = raw[pos + 4 : pos + 8]
        body = raw[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width = int.from_bytes(body[0:4], "big")
            height = int.from_bytes(body[4:8], "big")
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length
    if not idat or not width:
        return None
    try:
        data = zlib.decompress(bytes(idat))
    except zlib.error:
        return None
    # Sample rather than decode every scanline: filters make exact reconstruction
    # costly, and for "one flat colour or not" a sample is sufficient and honest
    # about being one.
    step = max(1, len(data) // 60000)
    sample = data[::step]
    colours = len(set(sample[i : i + 3] for i in range(0, len(sample) - 3, 3)))
    return {
        "size": f"{width}x{height}",
        "colours": colours,
        "digest": zlib.crc32(sample),
        "bytes": path.stat().st_size,
    }


first = png_stats(work / "first.png")
second = png_stats(work / "second.png")


def read_focus(name: str) -> str:
    path = work / name
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8", errors="replace").strip()
    match = re.search(r"(?:mCurrentFocus=Window|ResumedActivity: ActivityRecord)\{\S+ \S+ ([^} ]+)", text)
    return match.group(1) if match else text


focus1, focus2 = read_focus("focus1.txt"), read_focus("focus2.txt")
# The activity the caller named, without the package prefix, as dumpsys spells it.
wanted = activity.split("/", 1)[1] if "/" in activity else activity
wanted_full = wanted if wanted.startswith(".") is False else pkg + wanted
focus_known = bool(focus1)
focus_is_wanted = focus_known and (wanted_full in focus1 or wanted in focus1)
focus_in_pkg = focus_known and pkg in focus1

log = ""
log_path = work / "logcat.txt"
if log_path.exists():
    log = log_path.read_text(encoding="utf-8", errors="replace")

# Content-side failures, in the forms this runtime emits them. Ordered most to
# least specific, because a generic match would swallow the useful one.
EXCEPTION_PATTERNS = [
    (r"Uncaught\b.*", "uncaught JS exception"),
    (r"\bmigo_[a-z_]*error\b.*", "C ABI error"),
    (r"module evaluation failed.*", "module evaluation failed"),
    (r"\bReferenceError\b.*", "ReferenceError"),
    (r"\bTypeError\b.*", "TypeError"),
    (r"\bSyntaxError\b.*", "SyntaxError"),
    (r"not supported\b.*", "unsupported call"),
    (r"\bFATAL EXCEPTION\b.*", "host crash"),
]
# Did a session actually run? The Java SDK logs its lifecycle at default level,
# with no configuration, on any host that uses it -- which makes it the one
# host-agnostic signal that the engine was handed this content at all.
#
# Pixels cannot supply this. An earlier version of this tool was pointed at a
# host's menu screen, saw a colourful surface belonging to the host's own UI, and
# reported "Runs" for a bundle that was never loaded. A verdict that can be
# reached without the engine ever starting is not a verdict about the engine.
session_running = bool(re.search(r"MigoSession.*state:.*->\s*RUNNING", log))
session_seen = bool(re.search(r"\bMigoSession\b", log))

found: list[tuple[str, str]] = []
seen: set[str] = set()
for pattern, label in EXCEPTION_PATTERNS:
    for match in re.finditer(pattern, log):
        line = match.group(0).strip()[:200]
        if line in seen:
            continue
        seen.add(line)
        found.append((label, line))

painted = bool(first and first["colours"] > 8)
moving = bool(first and second and first["digest"] != second["digest"])
crashed = any(label == "host crash" for label, _ in found)
running = bool(alive)

out: list[str] = []
out.append(f"# Prescreen (run): `{pathlib.Path(bundle).name}`")
out.append("")
out.append(f"- host: `{pkg}` — `{activity}`")
out.append(f"- device: {model or 'unknown'} (API {sdk or '?'})")
out.append(f"- observed for {secs}s")
out.append(f"- window in front at capture: `{focus1 or 'unknown'}`")
scope_path = work / "log-scope.txt"
log_scope = scope_path.read_text(encoding="utf-8").strip() if scope_path.exists() else "unknown"
out.append(f"- log scope: {log_scope}")
out.append(
    f"- engine session reached RUNNING: {'yes' if session_running else 'no'}"
    + ("" if session_seen else " (no `MigoSession` line at all)")
)
if focus2 and focus2 != focus1:
    out.append(f"- and at the second capture: `{focus2}`")
out.append("")
if not focus_known:
    out.append(
        "> **Could not read which window was in front.** The sections below stand "
        "on their own, but nothing here confirms the host owned the screen."
    )
    out.append("")
elif not focus_in_pkg:
    out.append(
        "> **The host was not the window in front.** Everything below describes "
        "whatever was, so none of it is evidence about this bundle."
    )
    out.append("")
elif not focus_is_wanted:
    out.append(
        f"> **The window in front is not the activity that was asked for** "
        f"(`{wanted}`). It belongs to the host, so the host is running — but a "
        "screenshot of a host's own UI says nothing about the bundle."
    )
    out.append("")

# --- 1. did it paint ---
out.append("## 1. Did it put a frame on screen")
out.append("")
if first is None:
    out.append("**No screenshot could be captured.** Nothing can be said about rendering.")
else:
    out.append(f"| | frame 1 (t≈{int(secs)//2}s) | frame 2 (t≈{secs}s) |")
    out.append("|---|---|---|")
    out.append(f"| distinct colours sampled | {first['colours']} | {second['colours'] if second else 'n/a'} |")
    out.append(f"| surface | {first['size']} | {second['size'] if second else 'n/a'} |")
    out.append("")
    if painted and moving:
        out.append("**Painted, and still changing between the two captures.**")
    elif painted:
        out.append(
            "**Painted, but the two captures are identical.** Either the scene is "
            "static by then, or it stopped. Look at the PNGs before reading this "
            "as a stall."
        )
    else:
        out.append(
            "**Effectively one flat colour — nothing recognisable was drawn.** "
            "A game that legitimately opens on a solid fill reads the same way, so "
            "check the PNG before calling this a failure."
        )
    out.append("")
    out.append(f"Evidence: `{report_dir}/prescreen-frame-1.png`, `{report_dir}/prescreen-frame-2.png`")
out.append("")

# --- 2. what the log said ---
out.append("## 2. Content-side errors")
out.append("")
if not log:
    out.append("No log was captured, so this section proves nothing either way.")
elif not found:
    out.append(
        "None matched. That is not the same as none happening — this greps for the "
        "shapes this runtime emits, and content can fail quietly."
    )
else:
    out.append(f"{len(found)} distinct line(s):")
    out.append("")
    for label, line in found[:40]:
        out.append(f"- **{label}** — `{line}`")
    if len(found) > 40:
        out.append(f"- ... and {len(found) - 40} more")
out.append("")

# --- 3. verdict ---
out.append("## 3. Verdict")
out.append("")
blockers = [f"{label}: `{line}`" for label, line in found
            if label in {"host crash", "module evaluation failed", "uncaught JS exception"}]
if focus_known and not focus_in_pkg:
    verdict = "**Nothing observed**"
    why = (
        f"`{focus1}` held the foreground, not the host, so the screen and the log "
        "describe something else. Check that the activity exists and starts."
    )
elif focus_known and not focus_is_wanted:
    verdict = "**Could not confirm the bundle was on screen**"
    why = (
        f"the host is in front, but showing `{focus1}` rather than the activity "
        f"named (`{wanted}`) — a host's menu, most likely. Point --activity at the "
        "window that hosts the game surface. Refusing here is deliberate: the "
        "first version of this tool called that case \"Runs\"."
    )
elif crashed or not running:
    verdict = "**Not runnable as-is**"
    why = "the host process was gone before the observation window ended."
elif blockers:
    verdict = "**Blocked**"
    why = "it started, and content-side failures were logged."
elif not session_running:
    verdict = "**No engine session ran**"
    if session_seen:
        why = (
            "the host was on screen and the SDK logged session activity, but no "
            "session reached RUNNING. The bundle was not started."
        )
    else:
        why = (
            "nothing on screen came from a Migo session — there is no `MigoSession` "
            "line in the log at all. Either the activity named is not the one that "
            "hosts the game (a menu screen looks just as alive in a screenshot), or "
            "this host embeds the engine through the C ABI without the Java SDK, "
            "which logs no lifecycle. Either way, the pixels below are not evidence "
            "about this bundle."
        )
elif painted and moving:
    verdict = "**Runs**"
    why = f"a session ran, and it painted a changing scene for {secs}s with no logged content failure."
elif painted:
    verdict = "**Runs, with a caveat**"
    why = "a session ran and painted, but the scene stopped changing. Check the frames."
else:
    verdict = "**Inconclusive**"
    why = "the process stayed up and logged nothing, but nothing recognisable was drawn."
out.append(f"{verdict} — {why}")
out.append("")
if blockers:
    out.append("Blockers:")
    out.append("")
    for item in blockers[:20]:
        out.append(f"- {item}")
    out.append("")

out.append("## What this is not")
out.append("")
out.append(
    f"- **Not a certification.** {secs} seconds, one device, one launch. It says "
    "nothing about level two, about a cold cache, or about a device you do not own."
)
out.append(
    "- **Not the whole prescreen.** Run `scripts/prescreen-game.sh` as well: this "
    "half sees what broke, that half sees which APIs the bundle needs and whether "
    "this build has them. Neither answers the other's question."
)
out.append(
    "- **Not an instrumented measurement.** It watches the screen and the log, "
    "because the bundle belongs to someone else and was not built to be watched."
)

print("\n".join(out))
PY

if [[ -n "$OUT" ]]; then
    say "report -> $OUT"
fi
