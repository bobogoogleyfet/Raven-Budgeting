#!/usr/bin/env bash
#
# Copy the built web app into a local clone of the site repository, ready to
# commit and push. Stops short of both: it prints the commands instead.
#
#   scripts/publish-web.sh ../raven-site
#
# Build first with scripts/build-web.sh. The site repository is the public one
# GitHub Pages serves from — `<org>.github.io` in an organisation that hosts
# nothing else (see "The web app" in the README) — and holds the built site
# and nothing more. Everything in the clone except .git is replaced.
#
# Refuses, before touching anything, when:
#   - the target is not the top of a git clone, or has uncommitted or ignored
#     files (everything it removes must be recoverable from its history);
#   - the target is this repository, or inside it, or contains it;
#   - the target holds source code, which means it is the wrong repository;
#   - anything in the target or the build looks like data — a database, a
#     sync file, an export. A public repository is the one place that must
#     never hold any.
#
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$REPO/app/build/web"

usage() { sed -n '3,21p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; }
case "${1:-}" in
  -h|--help) usage; exit 0 ;;
esac
[ $# = 1 ] || { usage >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }

[ -f "$BUILD/index.html" ] || die "no build in $BUILD; run scripts/build-web.sh first."
# Left by build-web.sh only when its checks passed. Anything newer than it in
# the build was put there by hand afterwards, and was never checked.
CHECKED="$REPO/app/build/web.checked"
[ -f "$CHECKED" ] ||
  die "$BUILD was not built by scripts/build-web.sh, or failed its checks. Build with that script."
[ -z "$(find "$BUILD" -newer "$CHECKED" -print -quit)" ] ||
  die "$BUILD has changed since scripts/build-web.sh checked it. Build again."
[ -d "$1" ] || die "$1 is not a directory."
SITE="$(cd "$1" && pwd -P)"
HERE="$(cd "$REPO" && pwd -P)"

# ---------------------------------------------------------------------------
# Is this the right place?
# ---------------------------------------------------------------------------
top="$(git -C "$SITE" rev-parse --show-toplevel 2>/dev/null)" ||
  die "$SITE is not a git clone. Clone the site repository and pass its directory."
[ "$(cd "$top" && pwd -P)" = "$SITE" ] ||
  die "$SITE is inside the clone at $top; pass the top of the clone."
case "$SITE/" in "$HERE/"*) die "$SITE is this repository, or inside it." ;; esac
case "$HERE/" in "$SITE/"*) die "$SITE contains this repository." ;; esac
# Ignored files too: they are not in the history either, so deleting one
# would be deleting it for good.
[ -z "$(git -C "$SITE" status --porcelain --untracked-files=all --ignored)" ] ||
  die "$SITE has uncommitted or ignored files. Commit or remove them first: everything but .git is about to be replaced."
for marker in Cargo.toml pubspec.yaml app crates; do
  [ ! -e "$SITE/$marker" ] ||
    die "$SITE has $marker in it, which is source code, not a built site. Is this the right repository?"
done

# Data, anywhere in either tree. Tracked files are checked by name as well as
# on disk, so a database deleted from the working copy but still committed is
# found too — it would still be public in the history, and that needs a human.
data_like() {
  grep -iE '\.(db|db-wal|db-shm|db-journal|sqlite|sqlite3|paccsync|xlsx|csv)$' || true
}
found="$(
  {
    (cd "$SITE" && find . -path ./.git -prune -o -type f -print)
    git -C "$SITE" ls-files
    (cd "$BUILD" && find . -type f)
  } | data_like
)"
[ -z "$found" ] || die "refusing: these look like data, and a public repository must never hold any:
$found"

# ---------------------------------------------------------------------------
# Replace the site
# ---------------------------------------------------------------------------
find "$SITE" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp -R "$BUILD/." "$SITE/"
# GitHub Pages runs Jekyll over the site unless told not to, and Jekyll drops
# anything whose name starts with an underscore or a dot. Nothing here needs
# processing, and a file that silently fails to publish is a broken app.
touch "$SITE/.nojekyll"

source_rev="$(git -C "$REPO" rev-parse --short HEAD)"
[ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] || source_rev="$source_rev with uncommitted changes"

cat <<EOF
==> $SITE now holds the site built from $source_rev.

Nothing has been committed or pushed. Look before you do:

  cd $SITE
  git add -A
  git status        # only built files: index.html, main.dart.js, canvaskit/, pkg/, ...
  git commit -m "Publish $source_rev"
  git push
EOF
