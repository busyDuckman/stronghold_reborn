#!/usr/bin/env bash
# Shared shell helpers. Source this at the top of any scripts/*.sh:
#
#     source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"
#
# Single source of truth for banners, logging and dependency checks, so the
# three-projects-worth of copy-paste drift does not happen again.
# Stdlib bash only; safe to source before any tooling exists.

# ----------------------------------------------------------------------------------------------------------------------
# logging

log()  { printf '[INFO] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*" >&2; }
err()  { printf '[ERROR] %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }

# Echo a command then run it. Use for anything a reader would want to reproduce by hand.
run()  { printf '[CMD ] %s\n' "$*"; "$@"; }

# Hard-fail if a required binary is missing. No silent fallbacks.
need() { command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"; }

# ----------------------------------------------------------------------------------------------------------------------
# banner

# Print a centred, coloured banner. Colors: red, orange, green, blue (default orange).
#   banner "Some message" green
banner() {
    local msg="${1:-}"
    local c r bar width msglen spaces
    case "${2:-orange}" in
        red)    c=$'\033[1;38;5;196m' ;;
        orange) c=$'\033[1;38;5;208m' ;;
        green)  c=$'\033[1;38;5;46m'  ;;
        blue)   c=$'\033[1;38;5;33m'  ;;
        *)      c=$'\033[1m' ;;
    esac
    r=$'\033[0m'
    bar='===================================================================='
    width=${#bar}
    msglen=${#msg}
    if (( msglen < width )); then
        spaces=$(printf '%*s' "$(( (width - msglen) / 2 ))" '')
    else
        spaces=''
    fi
    printf '%s%s%s\n' "$c" "$bar" "$r"
    printf '%s%s%s%s\n' "$c" "$spaces" "$msg" "$r"
    printf '%s%s%s\n' "$c" "$bar" "$r"
}

# ----------------------------------------------------------------------------------------------------------------------
# paths

# Absolute path of the repo root (the parent of scripts/). Scripts anchor their
# relative paths on this so they work from any cwd.
repo_root() {
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &>/dev/null && pwd
}

# cd to the repo root. Most scripts want this as their first real statement.
cd_repo_root() { cd "$(repo_root)" || die "cannot cd to repo root"; }

# The project's python. Everything python goes through the venv; never system
# or --user packages. Created by `just setup`.
venv_python() {
    local root; root="$(repo_root)"
    if   [ -x "$root/.venv/bin/python3" ];        then echo "$root/.venv/bin/python3"
    elif [ -x "$root/.venv/Scripts/python.exe" ]; then echo "$root/.venv/Scripts/python.exe"
    else die "no project .venv found; run: just setup"
    fi
}

# ----------------------------------------------------------------------------------------------------------------------
# env files

# Load a gitignored KEY=VALUE env file if it exists. Never fails when absent,
# but the caller must still check that the vars it needed actually arrived.
#   load_env .upstash.env
load_env() {
    local f="$(repo_root)/$1"
    [ -f "$f" ] || return 0
    set -a
    # shellcheck disable=SC1090
    source "$f"
    set +a
}
