#!/usr/bin/env bash
# No published runtime API may decide its answer with a coin flip.
#
# `update/02_update_mgr.js` did. `Math.random() < 0.3` chose, at construction,
# whether content was told an update was waiting; a random 2-5 s later a second
# flip fired onUpdateReady (90%) or onUpdateFailed (10%). Roughly a quarter of
# launches showed the game's own "new version -- restart?" prompt, the player
# accepted, `applyUpdate()` logged "Application restarted with new version", and
# nothing restarted.
#
# The cost of that shape is not the missing feature -- this runtime has no update
# channel and saying so is fine. It is that the failure moved. Content met it on
# some launches and not others, in a different place each time, so it survives
# testing and appears in the field. And `checkUpdate()`, five lines further down
# the same file, had always answered `hasUpdate: false`: the truthful answer
# existed, the fabricating entry point was simply the one with no test and no
# @stub marker, so the prescreen report called it supported.
#
# Randomness itself is not the offence -- generating a unique identifier with it
# is ordinary. Deciding what to *tell content* with it is. The two are impossible
# to tell apart by pattern, so each use declares which it is, inline:
#
#     name = `__anon_${Date.now()}_${Math.random()}`;   // @random-ok unique id
#
# A marker rather than a list in this file, so the justification lives where the
# next reader meets the code, and writing one is a sentence you have to mean.
#
# Host-only: reads the embedded JS, runs nothing.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JS_ROOT="$ROOT/engine/crates/runtime-v8/src"

[[ -d "$JS_ROOT" ]] || { echo "FAIL: $JS_ROOT does not exist" >&2; exit 1; }

unjustified=()
checked=0

while IFS= read -r hit; do
    file="${hit%%:*}"
    rest="${hit#*:}"
    line="${rest%%:*}"
    text="${rest#*:}"
    # A line-comment mention (explaining a past one, as the update manager's
    # header now does) is prose, not a call, and must not be counted as one --
    # a gate that inflates its own coverage number is telling the same kind of
    # story this one exists to stop.
    [[ "$text" =~ ^[[:space:]]*(//|\*) ]] && continue
    checked=$((checked + 1))
    [[ "$text" == *"@random-ok"* ]] && continue
    unjustified+=("${file#"$ROOT/"}:$line")
done < <(grep -rn "Math\.random" --include=*.js "$JS_ROOT" || true)

if (( checked == 0 )); then
    echo "FAIL: no Math.random found anywhere under $JS_ROOT -- the scan is broken," >&2
    echo "      not the code. It has always had at least the AMD shim's." >&2
    exit 1
fi

if (( ${#unjustified[@]} > 0 )); then
    echo "FAIL: ${#unjustified[@]} use(s) of Math.random in the runtime with no justification." >&2
    printf '\n'
    printf '  %s\n' "${unjustified[@]}"
    cat >&2 <<'MSG'

  If this generates an identifier or jitters a retry, say so on the line:

      // @random-ok <why>

  If it decides what to tell content -- whether an update exists, whether a
  call succeeded, what a device reports -- then it is inventing an answer.
  Content will meet it on some launches and not others. Give the truthful
  answer for a runtime that lacks the capability, and mark the entry point
  @stub so the prescreen report says so.
MSG
    exit 1
fi

echo "PASS: every Math.random in the runtime declares why ($checked use(s) checked)"
