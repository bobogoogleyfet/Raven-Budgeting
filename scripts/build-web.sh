#!/usr/bin/env bash
#
# Build the web app into app/build/web, ready for scripts/publish-web.sh.
#
#   scripts/build-web.sh             build the Rust wasm, then the Flutter site
#   scripts/build-web.sh --no-bridge reuse the wasm already in app/web/pkg/
#
# Then check what came out. The page holds a household's whole ledger, and the
# one promise it makes is that nothing leaves the device: no request goes
# anywhere but the page's own origin. The Content-Security-Policy in
# web/index.html is what enforces that in the browser; the checks at the end
# are what stop a build from quietly depending on somewhere else — a Flutter
# upgrade that goes back to a CDN, a dependency that phones home — and
# shipping as a page that is either broken by its own policy or no longer
# carrying one.
#
# $FLUTTER names the flutter binary when it is not the one on $PATH.
#
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$REPO/app"
OUT="$APP/build/web"
FLUTTER="${FLUTTER:-flutter}"

BRIDGE=1
while [ $# -gt 0 ]; do
  case "$1" in
    --no-bridge) BRIDGE=0; shift ;;
    -h|--help) sed -n '3,17p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# The Rust half
# ---------------------------------------------------------------------------
# `flutter build web` does not build the bridge — cargokit does that on every
# other platform, and nothing does on this one — so it comes first, and the
# page loads what it leaves in web/pkg/.
if [ "$BRIDGE" = 1 ]; then
  [ -f "$REPO/scripts/build-web-bridge.sh" ] ||
    die "scripts/build-web-bridge.sh is missing. It compiles the Rust bridge to wasm into app/web/pkg/, and the web app cannot run without it."
  echo "==> building the Rust bridge for the web"
  bash "$REPO/scripts/build-web-bridge.sh"
fi
for f in accounting_bridge.js accounting_bridge_bg.wasm; do
  [ -s "$APP/web/pkg/$f" ] ||
    die "app/web/pkg/$f is missing — run scripts/build-web-bridge.sh (or this script without --no-bridge)."
done

# ---------------------------------------------------------------------------
# The Flutter half
# ---------------------------------------------------------------------------
# --no-web-resources-cdn: CanvasKit and Roboto from this origin, not gstatic.
# --csp: no code generated at run time, so script-src needs no 'unsafe-eval'.
# --no-source-maps: nothing to gain from publishing them.
# --base-href /: the site is the whole of its own origin (see the README).
# JavaScript rather than --wasm, which is not what the policy was checked with.
echo "==> building the Flutter web app"
# Written only once every check below has passed, and required by
# publish-web.sh, so a build made with plain `flutter build web` — or one that
# failed these checks — cannot be published by accident.
CHECKED="$APP/build/web.checked"
rm -rf "$OUT" "$CHECKED"
(
  cd "$APP"
  "$FLUTTER" build web --release \
    --no-web-resources-cdn \
    --csp \
    --no-source-maps \
    --no-wasm-dry-run \
    --base-href /
)

# Flutter still writes its service worker even though web/flutter_bootstrap.js
# never registers it, and a build id for its own incremental builds. A file
# nothing loads is one more thing to wonder about on a public site.
rm -f "$OUT/flutter_service_worker.js" "$OUT/.last_build_id"

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------
echo "==> checking the build"
problems=0
fail() { echo "  FAIL: $*" >&2; problems=$((problems + 1)); }

# The policy itself. Without it everything below is advice.
grep -q "http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';" "$OUT/index.html" ||
  fail "index.html has no Content-Security-Policy starting from default-src 'none'"
grep -q "connect-src 'self';" "$OUT/index.html" ||
  fail "index.html's policy does not limit connections to the page's own origin"
grep -q '<meta name="referrer" content="no-referrer">' "$OUT/index.html" ||
  fail "index.html does not set referrer to no-referrer"

# The two places the engine would reach for Google, and the settings that stop
# it. Both URLs are still in the shipped JavaScript as defaults; these are what
# make them dead code.
grep -q '"useLocalCanvasKit":true' "$OUT/flutter_bootstrap.js" ||
  fail "flutter_bootstrap.js loads CanvasKit from www.gstatic.com (built without --no-web-resources-cdn?)"
grep -q "fontFallbackBaseUrl: 'fonts/'" "$OUT/flutter_bootstrap.js" ||
  fail "flutter_bootstrap.js does not point font fallback at this origin; the engine would fetch from fonts.gstatic.com"
# Only what follows the build config is ours; flutter.js above it mentions
# service workers whether or not one is used.
if sed -n '/^_flutter.buildConfig = /,$p' "$OUT/flutter_bootstrap.js" | grep -q serviceWorker; then
  fail "flutter_bootstrap.js registers a service worker"
fi

# Every absolute URL in anything the page loads as text. Each host is either
# documentation that only ever appears in an error message or a comment, or
# one of the two defaults disarmed above, in the one file it is expected in;
# anything else is new and has to be looked at before it ships. WebAssembly is
# left out: CanvasKit's binaries carry a compression dictionary full of
# URL-shaped noise, and code in them reaches the network only through
# JavaScript, which is scanned.
allowed() { # <file, relative to the build> <host>
  case "$2" in
    github.com | web.dev | wicg.github.io | pub.dev | \
    flutter.dev | docs.flutter.dev | api.flutter.dev) return 0 ;;
    www.gstatic.com) [ "$1" = flutter.js ] || [ "$1" = flutter_bootstrap.js ] ;;
    fonts.gstatic.com) [ "$1" = main.dart.js ] ;;
    *) return 1 ;;
  esac
}
while IFS= read -r hit; do
  file="${hit%%:*}"; file="${file#"$OUT"/}"
  url="${hit#*:}"
  allowed "$file" "${url#*://}" ||
    fail "$file refers to $url, which nothing in this site should know about"
done < <(
  find "$OUT" -type f \( -name '*.html' -o -name '*.js' -o -name '*.mjs' \
      -o -name '*.json' -o -name '*.css' \) -print0 |
    xargs -0 grep -HoE 'https?://[A-Za-z0-9.-]*[A-Za-z0-9]' |
    sort -u
)

# Nothing personal can be in a fresh build — which is exactly why finding
# something shaped like it means a stray file found its way into web/.
while IFS= read -r stray; do
  fail "${stray#"$OUT"/} looks like data, not code, and must never be published"
done < <(find "$OUT" -type f \( -iname '*.db' -o -iname '*.db-*' -o -iname '*.sqlite*' \
    -o -iname '*.paccsync' -o -iname '*.xlsx' -o -iname '*.csv' \))
while IFS= read -r map; do
  fail "${map#"$OUT"/} is a source map"
done < <(find "$OUT" -type f -name '*.map')

[ "$problems" = 0 ] || die "$problems problem(s) with the build in $OUT; nothing there should be published."
date -u +%FT%TZ > "$CHECKED"
echo "==> $OUT is ready: scripts/publish-web.sh <your clone of the site repository>"
