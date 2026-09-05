#!/usr/bin/env bash
# A failed JNI field read must not be absorbed into a default.
#
# THE DRIFT THIS EXISTS TO CATCH shipped, and it cost several sessions of
# investigation before anybody found it. `engine/crates/platform/src/android/jni`
# reads `RuntimeConfig` field by field through helpers that return a `Result`
# carrying the field name and the reason it failed. Every caller wrote
# `.unwrap_or(some_default)`, which is correct-looking and throws the reason
# away.
#
# For a Java primitive or an enum reference that is never right. Those fields
# ALWAYS HAVE A VALUE, so `env.get_field` cannot fail because the host declined
# to set one -- it fails when the field is not on the class this code names,
# which means the host was built against a different SDK than the library it
# loaded. Absorbed into a default, a version mismatch is indistinguishable from
# a host that chose that default.
#
# The concrete cost: a host calling `setLogLevel(INFO)` could be read as WARN,
# and the only evidence either way was the absence of the logs it asked for --
# which is also what a working INFO run looks like before its first frame. The
# engine's own boundary-crossing counter, added specifically to measure a claim
# on a device, has never been observed working there.
#
# So the rule is not "handle every error"; it is narrower and checkable: the
# readers whose failure can only mean a mismatch must not be silently defaulted.
# String fields are deliberately excluded -- a null String IS a legitimate
# "unset", and `get_optional_string_field` exists to say so.
#
# This lane exists because the compiler cannot help here:
# `platform/src/android/jni/**` is `#[cfg(target_os = "android")]`, so nothing in
# it is ever executed by a test on any machine in this project. It is compiled
# for the target and read by people.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

JNI_DIR="engine/crates/platform/src/android/jni"

# The readers whose failure means "this field is not on that class".
READERS='get_i32|get_bool|get_f32|get_enum_ordinal'
# The ways a Result is turned into a value without looking at the error.
#
# Bracket classes and not backslash escapes: this pattern is handed to both
# `grep -E` and `awk`, and awk reads `\.` in a string as a plain `.` -- which is
# the regex for "any character". The multi-line half of the detector would still
# have matched things and would have stopped matching the thing it names, with
# only a warning on stderr to say so. A detector that quietly means something
# else is the failure mode this whole gate is about.
SWALLOWS='unwrap_or|unwrap_or_default|unwrap_or_else|ok[(][)]|unwrap[(][)]'

problems=()
notes=()

[[ -d "$JNI_DIR" ]] || { echo "FAIL: $JNI_DIR does not exist; this gate inspected nothing" >&2; exit 1; }

mapfile -t sources < <(find "$JNI_DIR" -name '*.rs' -type f | sort)
if (( ${#sources[@]} == 0 )); then
    problems+=("no Rust sources under $JNI_DIR; this gate inspected nothing")
else
    notes+=("scanned ${#sources[@]} source(s) under $JNI_DIR")
fi

# The control. The detector has to be able to see the pattern it is looking for,
# or a clean tree and a broken regex read identically -- which is the same class
# of mistake this gate is about.
control="$(printf 'let x = super::get_i32(&mut env, "f", &o).unwrap_or(3);\n')"
if ! printf '%s\n' "$control" | grep -qE "($READERS)[(].*[)][[:space:]]*[.]($SWALLOWS)"; then
    echo "FAIL: the detector does not match a known-bad line, so its clean result" >&2
    echo "      for $JNI_DIR means nothing." >&2
    exit 1
fi
notes+=("control: the detector matches a known-bad line")

# Same-line form, and the wrapped form where the swallow starts the next line.
hits="$(
    {
        grep -nE "($READERS)[(].*[)][[:space:]]*[.]($SWALLOWS)" ${sources[@]+"${sources[@]}"} || true
        # `get_x(` ... `)` on one line and `.unwrap_or(` on the next: joined with
        # awk rather than a multi-line regex so the reported line is the call.
        awk -v readers="$READERS" -v swallows="$SWALLOWS" '
            prev ~ ("(" readers ")[(]") && $0 ~ ("^[[:space:]]*[.](" swallows ")") {
                # FNR and not NR: NR keeps counting across the whole file
                # list, so on every file but the first it names a line that
                # does not exist. The finding was right and the address was
                # wrong, which is the harder kind of wrong to notice.
                printf "%s:%d:%s\n", FILENAME, FNR - 1, prev
            }
            # Reset at each file boundary, or the first line of one file pairs
            # with the last line of the previous one.
            FNR == 1 { prev = "" }
            { prev = $0 }
        ' ${sources[@]+"${sources[@]}"}
    } | sort -u
)"

if [[ -n "$hits" ]]; then
    problems+=("a JNI field read is defaulted without reporting why it failed:
$(printf '%s\n' "$hits" | sed 's/^/        /')")
fi

printf '\n'
for note in ${notes[@]+"${notes[@]}"}; do echo "  - $note"; done
printf '\n'

if (( ${#problems[@]} > 0 )); then
    echo "FAIL: a failed JNI field read is being absorbed into a default." >&2
    printf '\n' >&2
    for problem in ${problems[@]+"${problems[@]}"}; do echo "  * $problem" >&2; done
    cat >&2 <<'WHY'

  Use `super::or_default(read, fallback)` instead. It returns the same value and
  says, at WARN, which field could not be read and why -- WARN because that is
  the release default level, so the message is audible in exactly the builds
  where the mismatch would otherwise be invisible.

  If the field is genuinely optional, it is a String and belongs to
  `get_optional_string_field`, whose `None` means "unset" and not "unreadable".
WHY
    exit 1
fi

echo "PASS: every JNI primitive-field read reports a failure instead of defaulting through it."
