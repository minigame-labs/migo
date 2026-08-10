#!/usr/bin/env bash
# The timestamp a shipped artifact may carry.
#
# A release artifact that records when it was built cannot be rebuilt from the same
# source and compared: the bytes differ every run, which is the property Phase 1
# asks for and which `SOURCE_DATE_EPOCH` exists to give. The convention is not this
# repository's invention -- it is the reproducible-builds standard, honoured by
# tar, gzip, Gradle and rustc -- and the rule is: when it is set, it *is* the build
# time; when it is not, a wall clock is fine because nothing is being reproduced.
#
# Two things this replaces, both of which shipped:
#
#   * `build-aar.sh` wrote `"sourceDateEpoch": <the epoch>` and then
#     `"buildTime": "<local wall clock>"` on the very next line -- the input for
#     reproducibility recorded, and unused for the one field that defeated it. It
#     was also local time, so the same source in two timezones produced different
#     bytes.
#   * `write-snapshot-manifest.sh` stamped `generated_at` into manifests that are
#     *committed*, so a regeneration always diffs and the tracked file cannot be
#     reproduced from the source it describes.
#
# UTC always. A local timestamp is not a fact about the artifact, it is a fact
# about the machine, and two machines building identical bytes must not disagree
# about what they built.

# Emit an ISO-8601 UTC timestamp: `SOURCE_DATE_EPOCH` if set, else now.
#
# The caller is expected to have validated `SOURCE_DATE_EPOCH` as non-negative
# integer seconds; this refuses rather than silently falling back to now, because a
# malformed value means the caller believes it is producing a reproducible artifact
# and it is not.
reproducible_timestamp() {
    if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
        date -u +%Y-%m-%dT%H:%M:%SZ
        return
    fi
    if [[ ! "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
        echo "SOURCE_DATE_EPOCH must be non-negative Unix seconds, got: $SOURCE_DATE_EPOCH" >&2
        return 1
    fi
    # `-d @N` is GNU, `-r N` is BSD. Both are tried because this runs on developer
    # machines as well as Linux CI, and a wrong-but-successful date would be worse
    # than either.
    date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
        || date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ
}
