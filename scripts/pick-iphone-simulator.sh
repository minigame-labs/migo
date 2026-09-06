#!/usr/bin/env bash
# Print the udid of the newest available iPhone simulator on this machine.
#
# The simulator is CHOSEN, not named. A hardcoded `name=iPhone 16` is one runner-
# image bump away from failing for a reason that has nothing to do with this
# repository, so the destination is the udid of whichever iPhone runtime the
# image actually carries, newest first. If it carries none, that is a fact about
# the image and this says so.
#
# Extracted from .github/workflows/apple-sdk.yml on 2026-09-06, when a second
# step needed the same answer: the ANGLE the simulator tests load has to be put
# on the loader path of the device those tests will run on, which means both
# steps have to agree on which device that is. Two copies of a selection rule
# are two rules.
#
# Host-only, and macOS-only: it asks `xcrun simctl`.
set -euo pipefail

xcrun simctl list devices available --json | python3 -c '
import json, re, sys

runtimes = json.load(sys.stdin)["devices"]

# Sort key from com.apple.CoreSimulator.SimRuntime.iOS-18-5. Parsed rather than
# compared as text: iOS-9-0 sorts after iOS-18-5 lexicographically, which would
# pick the oldest runtime on an image carrying both.
def version(identifier):
    digits = re.findall(r"\d+", identifier.rsplit(".", 1)[-1])
    return tuple(int(d) for d in digits)

best = None
for identifier, devices in runtimes.items():
    if ".SimRuntime.iOS-" not in identifier:
        continue
    for device in devices:
        if not device.get("isAvailable"):
            continue
        if not device.get("name", "").startswith("iPhone"):
            continue
        # Newest runtime wins; within one runtime the highest device name wins.
        # That tie-break is arbitrary and it is meant to be: every simulated
        # iPhone runs the same ABI, so there is nothing to prefer, and an
        # arbitrary rule that is deterministic beats a preference nobody can
        # justify. The caller logs which device it picked.
        key = (version(identifier), device["name"])
        if best is None or key > best[0]:
            best = (key, device["udid"], identifier, device["name"])

if best is None:
    sys.exit("this runner image carries no available iPhone simulator")

sys.stderr.write("selected " + best[3] + " on " + best[2] + "\n")
print(best[1])
'
