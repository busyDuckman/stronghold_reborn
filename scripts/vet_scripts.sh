#!/usr/bin/env bash
# Parse-check the justfile and every .sh / .py under scripts/. Catches syntax
# errors before they bite halfway through a long `just vet` or a CI run.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"
cd_repo_root

just --summary > /dev/null

shopt -s nullglob globstar
for f in scripts/**/*.sh; do
    bash -n "$f" || die "bash syntax error in $f"
done
for f in scripts/**/*.py; do
    python3 -m py_compile "$f" || die "python syntax error in $f"
done

echo "vet-scripts: clean"
