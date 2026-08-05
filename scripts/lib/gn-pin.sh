# shellcheck shell=bash
# Reading and asserting the pinned `gn` identity.
# Location: scripts/lib/gn-pin.sh
#
# Sourced by scripts/build-gn.sh (to build the pinned gn) and by the V8 build
# scripts (to refuse a gn that is not the pinned one). One reader so the two
# cannot disagree about what the lock says.
#
# gn reports itself as `<version> (<short-revision>)`, e.g. `2502 (17b0057970fa)`.
# Neither half is an identity, and it is worth being precise about why, because
# checking the revision looks like it closes the gap and does not. Both numbers
# come from the same `git describe HEAD` at build time, with no dirty marker, so a
# gn built from the pinned commit WITHOUT the required patch -- or with extra local
# edits -- reports exactly the same string as the intended binary. The version
# string is therefore only a cheap first filter. `gn_pin_assert_binary` is the
# real check: it reads the receipt scripts/build-gn.sh leaves beside the installed
# gn, which names the patch set the builder applied and binds it to that binary's
# own hash.
#
# There is deliberately no separate minimum-version check. V8 14.5 needs gn 2315
# or newer for path_exists(), but an exact version pin already refuses everything
# below it, so a minimum could never be the reason a gn was rejected. The 2315
# requirement is recorded in the lock's notes, where a rationale belongs.

_gn_pin_err() { printf '  ✗ %s\n' "$*" >&2; }

# Populates GN_PIN_VERSION, GN_PIN_REVISION, GN_PIN_PATCHES.
gn_pin_read() {
    local lock="$1"
    [[ -f "$lock" ]] || { _gn_pin_err "missing build lock: $lock"; return 1; }
    local fields
    fields="$(python3 - "$lock" <<'PY'
import json, sys

lock = json.load(open(sys.argv[1]))
try:
    gn = lock["gn"]
    print(gn["version"])
    print(gn["revision"])
    print(" ".join(gn["required_patches"]))
except KeyError as missing:
    sys.exit(f"build lock has no gn {missing}")
PY
)" || { _gn_pin_err "cannot read the gn pin from $lock"; return 1; }
    { read -r GN_PIN_VERSION
      read -r GN_PIN_REVISION
      read -r -a GN_PIN_PATCHES
    } <<<"$fields"
}

# Checks a `gn --version` string against the pin. gn_pin_read must have run.
gn_pin_assert_version() {
    local reported="$1"
    local version="${reported%% *}"
    local short="${reported#*(}"
    short="${short%)}"
    if [[ "$reported" != *"("*")"* || -z "$version" || "$short" == "$reported" ]]; then
        _gn_pin_err "cannot parse gn version string: $reported"
        return 1
    fi
    if [[ "$version" != "$GN_PIN_VERSION" ]]; then
        _gn_pin_err "gn $version is not the pinned version $GN_PIN_VERSION"
        return 1
    fi
    if [[ ! "$short" =~ ^[0-9a-f]{7,40}$ ]]; then
        # Without a length floor, `"$short"*` would treat `2502 ()` and `2502 (1)`
        # as valid prefixes of the pinned sha and accept them.
        _gn_pin_err "gn reported an implausible revision abbreviation: '$short'"
        return 1
    fi
    if [[ "$GN_PIN_REVISION" != "$short"* ]]; then
        _gn_pin_err "gn revision $short does not match the pinned $GN_PIN_REVISION"
        return 1
    fi
}

# Where the receipt for an installed gn lives.
gn_pin_receipt_path() { printf '%s/gn-receipt.json' "$(dirname "$1")"; }

# Records what the gn beside it was actually built from. gn_pin_read must have run.
#
# `gn --version` is not an identity: it prints a commit position taken from
# `git describe HEAD` with no dirty marker, so a gn built from the pinned commit
# but WITHOUT the required patch, or with extra local edits, reports exactly the
# same string as the intended binary. The receipt closes that by naming the patch
# set the builder applied and binding it to this binary's own hash.
gn_pin_write_receipt() {
    local gn="$1" patch_dir="$2"
    python3 - "$gn" "$(gn_pin_receipt_path "$gn")" "$GN_PIN_REVISION" "$patch_dir" \
        "${GN_PIN_PATCHES[@]}" <<'PY'
import hashlib, json, pathlib, sys

def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()

binary, receipt, revision, patch_dir, *patch_ids = sys.argv[1:]
pathlib.Path(receipt).write_text(
    json.dumps(
        {
            "gn_revision": revision,
            "binary_sha256": sha256(binary),
            "patches": [
                {"id": pid, "sha256": sha256(pathlib.Path(patch_dir) / f"{pid}.patch")}
                for pid in patch_ids
            ],
        },
        indent=2,
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
PY
}

# Verifies an installed gn is the pinned one, patch set included.
# gn_pin_read must have run.
gn_pin_assert_binary() {
    local gn="$1" patch_dir="$2"
    [[ -x "$gn" ]] || { _gn_pin_err "not an executable gn: $gn"; return 1; }
    gn_pin_assert_version "$("$gn" --version 2>/dev/null || true)" || return 1
    local receipt
    receipt="$(gn_pin_receipt_path "$gn")"
    [[ -f "$receipt" ]] || {
        _gn_pin_err "no build receipt beside $gn"
        _gn_pin_err "gn --version cannot prove the pinned patches were applied;"
        _gn_pin_err "build it with ./scripts/build-gn.sh"
        return 1
    }
    python3 - "$gn" "$receipt" "$GN_PIN_REVISION" "$patch_dir" "${GN_PIN_PATCHES[@]}" <<'PY'
import hashlib, json, pathlib, sys

def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()

binary, receipt_path, revision, patch_dir, *patch_ids = sys.argv[1:]
try:
    receipt = json.loads(pathlib.Path(receipt_path).read_text(encoding="utf-8"))
except (OSError, ValueError) as error:
    sys.exit(f"unreadable gn receipt {receipt_path}: {error}")

if receipt.get("gn_revision") != revision:
    sys.exit(
        f"gn receipt records revision {receipt.get('gn_revision')}, "
        f"the lock pins {revision}"
    )
expected = [
    {"id": pid, "sha256": sha256(pathlib.Path(patch_dir) / f"{pid}.patch")}
    for pid in patch_ids
]
if receipt.get("patches") != expected:
    sys.exit(
        "gn receipt patch set differs from the lock: "
        f"receipt={receipt.get('patches')} lock={expected}"
    )
actual = sha256(binary)
if receipt.get("binary_sha256") != actual:
    sys.exit(
        f"gn binary {binary} hashes to {actual}, "
        f"its receipt records {receipt.get('binary_sha256')}"
    )
PY
}
