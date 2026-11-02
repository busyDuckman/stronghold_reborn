#!/usr/bin/env bash
# Scan tracked source files for AI-ism non-ASCII punctuation. The "usual suspects":
#   U+2013 en-dash, U+2014 em-dash, U+2018/U+2019 curly singles,
#   U+201C/U+201D curly doubles, U+2026 ellipsis, U+00A0 NBSP.
# These leak into the codebase via LLM output and break naive grep / search-replace
# in downstream pipelines. Use ASCII (-, --, ', ", ..., space).
#
# Usage: vet_chars.sh [ext1 ext2 ...]
#   - extensions are space-separated, no leading dot
#   - if no args, defaults to: rs py sh
# Per-line opt-out: put IGNORE_SMELL in a comment on the offending line.
set -uo pipefail

VET_CHARS_EXTS="${*:-rs py sh}"

pathspecs=()
for ext in $VET_CHARS_EXTS; do
    pathspecs+=("*.$ext")
done

# Curly singles are omitted from the class on purpose: too many false hits from
# prose apostrophes to be worth the noise. Add \x{2018}\x{2019} back if you disagree.
hits=$(git ls-files -z -- "${pathspecs[@]}" | xargs -0 -r grep -nHP --binary-files=without-match \
    '[\x{2013}\x{2014}\x{201C}\x{201D}\x{2026}\x{00A0}]' 2>/dev/null \
    | grep -vP '//\s*IGNORE_SMELL|#\s*IGNORE_SMELL' || true)

if [ -n "$hits" ]; then
    printf '%s\n\n' "$hits"
    echo "ERROR: forbidden non-ASCII punctuation found in the above lines." >&2
    echo "  U+2013 en-dash, U+2014 em-dash" >&2
    echo "  U+201C / U+201D curly doubles" >&2
    echo "  U+2026 ellipsis" >&2
    echo "  U+00A0 non-breaking space" >&2
    exit 1
fi
echo "vet-chars: clean (scanned: $VET_CHARS_EXTS)"
