#!/usr/bin/env bash
# Fail fast if VS Code is running with unsaved files. Without this, a long vet
# can pass against a stale working tree while the real edit sits in a backup
# buffer. Checks the editor's backup directory rather than guessing.
set -uo pipefail

if pgrep -x code > /dev/null 2>&1 || pgrep -x code-oss > /dev/null 2>&1; then
    unsaved=$(find "$HOME/.config/Code/Backups" -mindepth 3 -type f 2>/dev/null | wc -l)
    if [ "$unsaved" -gt 0 ]; then
        echo "ERROR: VS Code has $unsaved unsaved file(s); save all before running vet." >&2
        exit 1
    fi
fi
echo "vet-editor: no unsaved editor buffers"
