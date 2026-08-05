# shellcheck shell=bash
# Fail early, and by name, when a required tool is absent.
# Location: scripts/lib/host-requirements.sh
#
# Target is Linux only, so GNU patch, GNU coreutils and bash 5 are the baseline and
# are not probed -- a check whose failure case cannot arise on a supported host only
# adds noise. What does vary between Linux machines is which tools are installed: a
# lean CI image or container may carry no `patch` or no `python3`, and the failure
# then surfaces far from its cause.

host_requirements_err() { printf '  ✗ %s\n' "$*" >&2; }

# Names every missing tool rather than stopping at the first, so one run tells the
# operator everything they need to install.
require_host_tools() {
    local tool missing=0
    for tool in "$@"; do
        command -v "$tool" >/dev/null 2>&1 && continue
        host_requirements_err "$tool is required but not on PATH"
        missing=1
    done
    (( missing == 0 ))
}
