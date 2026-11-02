#!/usr/bin/env bash
# Thin CLI wrapper so `just banner` and non-bash callers can reach the banner
# helper. The body lives in _lib.sh; this file must stay a one-liner.
#   scripts/banner.sh "Some message" [red|orange|green|blue]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"
banner "${1:-}" "${2:-orange}"
