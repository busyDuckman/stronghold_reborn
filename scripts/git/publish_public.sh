#!/usr/bin/env bash
# Mirror the public part of this repo to the public repo, as one commit per push.
#
#     scripts/git/publish_public.sh                  # dry run: show what would be published, push nothing
#     scripts/git/publish_public.sh --push BEFORE    # CI: publish HEAD, message = every commit in BEFORE..HEAD
#
# The public tree is HEAD minus every tracked file that .gitignore with .extra_gitignore.public appended would
# ignore, and .gitignore itself is published with that file appended. It is built in a throwaway index, so the
# working tree and the real index are never touched. The public commit carries the private commit messages verbatim
# and the author of HEAD. If the public tree would not change, nothing is pushed.
#
# The ignore rules are read from the working tree. In CI that is HEAD; locally a dry run therefore also previews
# uncommitted edits to either ignore file.
#
# Pushing needs PUBLIC_REPO_TOKEN: a fine-grained token with Contents read/write on the public repo only. LFS objects
# go up through the git-lfs pre-push hook, so the checkout must have them (actions/checkout with lfs: true).
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../_lib.sh"

PUBLIC_REPO="busyDuckman/stronghold_reborn"
EXTRA_IGNORE=".extra_gitignore.public"
NULL_SHA="0000000000000000000000000000000000000000"

mode="dry"
before=""
case "${1:-}" in
    "")     ;;
    --push) mode="push"; before="${2:-}" ;;
    *)      die "usage: publish_public.sh [--push BEFORE_SHA]" ;;
esac

need git
cd_repo_root
[ -f "$EXTRA_IGNORE" ] || die "missing $EXTRA_IGNORE"

if [ "$mode" = "push" ]; then
    [ -n "${PUBLIC_REPO_TOKEN:-}" ] || die "PUBLIC_REPO_TOKEN is not set"
    url="https://x-access-token:${PUBLIC_REPO_TOKEN}@github.com/${PUBLIC_REPO}.git"
else
    url="https://github.com/${PUBLIC_REPO}.git"
fi

# ----------------------------------------------------------------------------------------------------------------------
# the public tree, built in a throwaway index

work="$(mktemp -d)" || die "mktemp failed"
trap 'rm -rf "$work"' EXIT

export GIT_INDEX_FILE="$work/index"
git read-tree HEAD || die "read-tree HEAD failed"

git ls-files -z -c -i --exclude-standard --exclude-from="$EXTRA_IGNORE" > "$work/held_back" \
    || die "listing the held back files failed"
git update-index -z --force-remove --stdin < "$work/held_back" || die "dropping the held back files failed"

gitignore_blob="$(cat .gitignore "$EXTRA_IGNORE" | git hash-object -w --stdin)" || die "hashing .gitignore failed"
git update-index --cacheinfo "100644,$gitignore_blob,.gitignore" || die "staging the public .gitignore failed"

tree="$(git write-tree)" || die "write-tree failed"
unset GIT_INDEX_FILE

log "held back from the public repo, by top-level folder:"
tr '\0' '\n' < "$work/held_back" | cut -d/ -f1 | sort | uniq -c

# ----------------------------------------------------------------------------------------------------------------------
# what the public repo has now

parent=""
if heads="$(git ls-remote "$url" refs/heads/main 2>/dev/null)"; then
    if [ -n "$heads" ]; then
        git fetch -q "$url" refs/heads/main || die "fetching the public main failed"
        parent="$(git rev-parse FETCH_HEAD)" || die "no FETCH_HEAD after fetch"
    fi
elif [ "$mode" = "push" ]; then
    die "cannot reach $PUBLIC_REPO (does it exist, and can PUBLIC_REPO_TOKEN write to it?)"
else
    warn "cannot reach $PUBLIC_REPO; previewing as if it were empty"
fi

if [ -n "$parent" ] && [ "$(git rev-parse "$parent^{tree}")" = "$tree" ]; then
    log "public tree unchanged; nothing to publish"
    exit 0
fi

if [ -n "$parent" ]; then
    log "changes against the public main:"
    git diff-tree -r --name-status "$parent" "$tree"
else
    log "the public repo is empty; the first commit would hold $(git ls-tree -r --name-only "$tree" | wc -l) files"
fi

if [ "$mode" = "dry" ]; then
    log "dry run: nothing pushed"
    exit 0
fi

# ----------------------------------------------------------------------------------------------------------------------
# commit and push

need git-lfs

if [ -n "$before" ] && [ "$before" != "$NULL_SHA" ] && git cat-file -e "$before^{commit}" 2>/dev/null; then
    message="$(git log --reverse --format='%B' "$before..HEAD")" || die "reading the commit messages failed"
else
    message="$(git log -1 --format='%B' HEAD)" || die "reading the commit message failed"
fi

GIT_AUTHOR_NAME="$(git log -1 --format='%an' HEAD)"
GIT_AUTHOR_EMAIL="$(git log -1 --format='%ae' HEAD)"
GIT_AUTHOR_DATE="$(git log -1 --format='%aI' HEAD)"
export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"

parent_args=()
[ -n "$parent" ] && parent_args=(-p "$parent")
commit="$(printf '%s\n' "$message" | git commit-tree "$tree" "${parent_args[@]}")" || die "commit-tree failed"

git lfs install --local >/dev/null || die "git lfs install failed"
git push -q "$url" "$commit:refs/heads/main" || die "push to $PUBLIC_REPO failed"
log "published $commit to $PUBLIC_REPO"
