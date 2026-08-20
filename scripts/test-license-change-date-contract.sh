#!/usr/bin/env bash
# The BSL Change Date decays, and nothing else in the tree notices.
#
# BSL 1.1 converts a version to the Change License on the Change Date **or** on
# the fourth anniversary of that version's first public distribution, whichever
# comes first. Two consequences follow, and both are silent:
#
#   * A Change Date further out than four years grants nothing. The Terms cap it,
#     so `2040-01-01` reads as protection the license does not actually give --
#     a claim a buyer's counsel will check and we would rather not have made.
#
#   * A **fixed** Change Date shrinks with every release made under it. Stamp
#     `2030-01-01` once and a release cut in 2029 carries one year, not four; a
#     release cut after it is born under the Change License. Nothing fails, no
#     test turns red, and the first person to notice is whoever reads the LICENSE
#     of a release that is already published.
#
# So the date is re-stamped every release as `publication + 4 years`, which is
# what MariaDB and HashiCorp ship, and this gate is what makes that true rather
# than merely intended. It does not compute the date or propose one: it checks
# that the stamped date is still a date, still in the future, and still inside
# the window a live re-stamping process would keep it in.
#
# The floor is what does the work. A tree whose Change Date is under three years
# out has not been re-stamped through a release in over a year, which is the
# decay above already in progress -- caught here, while it is an edit, instead of
# at the release that would ship it.
#
# The three-year floor is deliberately not four: the date is stamped once per
# release and then sits in the tree while development continues, so demanding a
# full four years would fail every commit made after release day. Three years
# leaves a year of ordinary drift and still fires long before a release could be
# cut with materially less than the term the README promises.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import datetime
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()
errors: list[str] = []

LICENSE = root / "LICENSE"
if not LICENSE.is_file():
    print("ERROR: LICENSE not found; there is no Change Date to check", file=sys.stderr)
    sys.exit(1)

text = LICENSE.read_text(encoding="utf-8")

# The parameter, not a mention of it. `Covenants of Licensor` ends with "To
# specify a Change Date", and the Terms name it twice more, so an unanchored
# search finds prose instead of the value.
match = re.search(r"^Change Date:\s*(.+)$", text, re.M)
if match is None:
    print(
        "ERROR: LICENSE has no `Change Date:` parameter line. BSL 1.1 requires one "
        "(Covenants of Licensor, item 3), and without it no version has a stated "
        "conversion date at all",
        file=sys.stderr,
    )
    sys.exit(1)

raw = match.group(1).strip()

# A bare ISO date, and nothing else. The parameter previously carried a paragraph
# of prose describing a rolling term, which reads fine and is unparseable -- so no
# check could tell whether it had decayed, which is the whole failure this gate
# exists for.
if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", raw):
    errors.append(
        f"`Change Date: {raw}` is not a bare YYYY-MM-DD date. A prose Change Date "
        "cannot be checked for decay, and decay is silent, so the parameter must be "
        "a date this gate can compare"
    )
    change_date = None
else:
    try:
        change_date = datetime.date.fromisoformat(raw)
    except ValueError:
        errors.append(f"`Change Date: {raw}` is not a real calendar date")
        change_date = None

if change_date is not None:
    today = datetime.date.today()

    def years_from(base: datetime.date, years: int) -> datetime.date:
        """`base` plus `years`, walking a 29 Feb base back to the 28th."""
        try:
            return base.replace(year=base.year + years)
        except ValueError:
            return base.replace(year=base.year + years, day=28)

    ceiling = years_from(today, 4)
    floor = years_from(today, 3)

    if change_date <= today:
        errors.append(
            f"Change Date {change_date} is not in the future (today is {today}). Any "
            "version released from this tree would be published already under the "
            "Change License -- Apache-2.0 on day one, with no BSL term at all"
        )
    elif change_date < floor:
        errors.append(
            f"Change Date {change_date} is less than three years out (today is {today}). "
            "It has not been re-stamped through a release in over a year, so the term is "
            f"decaying toward zero. Re-stamp it to the next release's publication date "
            f"plus four years -- for a release today that is {ceiling}"
        )

    if change_date > ceiling:
        errors.append(
            f"Change Date {change_date} is more than four years out (today is {today}, "
            f"four years is {ceiling}). BSL 1.1's Terms convert a version on the Change "
            "Date or its own fourth anniversary, whichever comes first, so the extra "
            "time is not granted by the license and stating it overstates the term"
        )

# The Change License is the other half of the parameter pair, and it is what
# LEGAL.md and both READMEs name. A Change Date is meaningless without it.
if re.search(r"^Change License:\s*Apache License 2\.0\s*$", text, re.M) is None:
    errors.append(
        "LICENSE's `Change License:` is no longer `Apache License 2.0`, which is what "
        "LEGAL.md and both READMEs tell readers the code converts to"
    )

# The public claims about the date. Each names it, and each was written by hand,
# so each can be left behind by an edit to LICENSE.
if change_date is not None:
    stated = change_date.isoformat()
    for relative in ("LEGAL.md", "README.md", "README.zh-CN.md"):
        path = root / relative
        if not path.is_file():
            errors.append(f"{relative} not found; it states the Change Date to readers")
            continue
        if stated not in path.read_text(encoding="utf-8"):
            errors.append(
                f"{relative} does not mention the stamped Change Date {stated}. It tells "
                "readers when the code converts, so it is a second copy of this fact and "
                "must move with the LICENSE"
            )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"License change date contract: FAIL ({len(errors)} violation(s))",
        file=sys.stderr,
    )
    sys.exit(1)

print(
    f"License change date contract: PASS (Change Date {change_date} converts to "
    f"Apache License 2.0; inside the four-year ceiling BSL 1.1 allows, and stated "
    f"identically in LEGAL.md and both READMEs)"
)
PY
