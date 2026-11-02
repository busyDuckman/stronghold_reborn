#!/usr/bin/env bash
# Print the result of the last CI run, loudly, as the final line of `just vet`.
#
# The beacon is an Upstash Redis REST key that the CI workflow SETs at the end of
# every run (see .github/workflows/ci.yml, "Report build status"). Reading it here
# costs one HTTP GET and means a red build cannot quietly sit there while you keep
# committing on top of it.
#
# Config lives in .upstash.env at the repo root (gitignored, see .upstash.env.example):
#   UPSTASH_URL=https://your-db.upstash.io
#   UPSTASH_READ_TOKEN=...        # read-only token; the CI workflow holds the write token
#   UPSTASH_KEY=LAST_CI_BUILD     # optional, this is the default
#
# No config = no beacon. That is a skip, not a failure: a project without CI wired
# up should not fail vet over it.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../_lib.sh"

load_env .upstash.env

UPSTASH_KEY="${UPSTASH_KEY:-LAST_CI_BUILD}"

if [ -z "${UPSTASH_URL:-}" ] || [ -z "${UPSTASH_READ_TOKEN:-}" ]; then
    echo "check-ci: no .upstash.env, skipping the CI beacon (see .upstash.env.example)"
    exit 0
fi

need curl
need jq

result=$(curl -sf "${UPSTASH_URL%/}/get/${UPSTASH_KEY}" \
    -H "Authorization: Bearer ${UPSTASH_READ_TOKEN}") || {
    banner "CI beacon unreachable (network? token?)" orange
    exit 0
}
status=$(echo "$result" | jq -r '.result // empty' 2>/dev/null)

case "$status" in
    PASSED) banner "Last CI build PASSED" green ;;
    FAILED) banner "YOUR LAST CI BUILD FAILED" red ;;
    *)      banner "No CI status recorded yet" orange ;;
esac
