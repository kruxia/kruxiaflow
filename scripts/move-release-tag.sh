#!/bin/bash
# Move a release tag to HEAD after fixing a failed tag-pipeline run.
#
# The situation this repairs: `cargo release X.Y.Z` pushed a bump commit and
# tag, CI failed on the tag run (fmt, clippy, a flaky test), and the fix is
# now committed on top. The tag must move to the fixed commit and be
# force-pushed so the release pipeline re-runs from scratch. The pipeline is
# idempotent (Docker/binaries overwrite, crates.io publish skips an
# already-published version), so re-running a moved tag is safe.
#
# Usage:
#   scripts/move-release-tag.sh [-y] [vX.Y.Z]
#
# With no tag argument, uses v<workspace version> from the root Cargo.toml.
# -y skips the confirmation prompt.
#
# Guardrails: clean working tree, on main, tag exists, tag version matches
# the workspace version at HEAD (the CI preflight enforces the same), and the
# tag isn't already at HEAD. Pushes main first so the tagged commit is on the
# remote branch, then force-pushes the tag.
set -euo pipefail

cd "$(dirname "$0")/.."

YES=false
TAG=""
for arg in "$@"; do
  case "$arg" in
    -y|--yes) YES=true ;;
    v*) TAG="$arg" ;;
    *) echo "error: unrecognized argument: $arg" >&2; exit 2 ;;
  esac
done

CARGO_VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
TAG=${TAG:-v$CARGO_VERSION}

# Untracked files are fine (they don't change the tagged tree); modified
# tracked files mean the fix isn't fully committed.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "error: tracked files have uncommitted changes — commit the fix first" >&2
  exit 1
fi

BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
  echo "error: on branch '$BRANCH' — release tags move only on main" >&2
  exit 1
fi

if ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "error: tag $TAG does not exist locally" >&2
  exit 1
fi

if [ "$TAG" != "v$CARGO_VERSION" ]; then
  echo "error: tag $TAG != workspace version $CARGO_VERSION at HEAD — CI preflight would fail" >&2
  exit 1
fi

OLD_SHA=$(git rev-parse "$TAG^{commit}")
NEW_SHA=$(git rev-parse HEAD)
if [ "$OLD_SHA" = "$NEW_SHA" ]; then
  echo "error: $TAG already points at HEAD ($(git rev-parse --short HEAD)) — nothing to move" >&2
  exit 1
fi

if ! git merge-base --is-ancestor "$OLD_SHA" "$NEW_SHA"; then
  echo "error: $TAG's current commit is not an ancestor of HEAD — refusing to move across histories" >&2
  exit 1
fi

# Preserve the original tag message (cargo-release creates annotated tags)
MSG=$(git tag -l --format='%(contents:subject)' "$TAG")
MSG=${MSG:-"release: $TAG"}

echo "Moving $TAG:"
echo "  from $(git log --oneline -1 "$OLD_SHA")"
echo "  to   $(git log --oneline -1 "$NEW_SHA")"
echo "  tag message: $MSG"
echo "Then: git push origin main && git push --force origin refs/tags/$TAG"

if [ "$YES" != "true" ]; then
  read -r -p "Proceed? [y/N] " REPLY
  case "$REPLY" in
    y|Y|yes|YES) ;;
    *) echo "aborted"; exit 1 ;;
  esac
fi

git tag -f -a -m "$MSG" "$TAG" HEAD
git push origin main
git push --force origin "refs/tags/$TAG"

echo
echo "$TAG moved and pushed — the release pipeline is re-running:"
echo "  https://github.com/kruxia/kruxiaflow/actions"
