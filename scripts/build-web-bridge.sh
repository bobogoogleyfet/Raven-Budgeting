#!/usr/bin/env bash
#
# Build the Rust half of the web app: the core and the bridge compiled to
# WebAssembly, plus the JavaScript glue that loads it, into app/web/pkg/.
#
#   scripts/build-web-bridge.sh           release build (what the site ships)
#   scripts/build-web-bridge.sh --debug   unoptimised, for chasing a panic
#
# `flutter build web` does not do this. On every other platform cargokit
# builds the bridge from inside Flutter's own build; on the web nothing does,
# so this runs first and `web/index.html` loads what it leaves behind:
# pkg/accounting_bridge.js and pkg/accounting_bridge_bg.wasm. Those are build
# output and are not committed.
#
# Needs, beyond the usual toolchain:
#   - the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown)
#   - clang, which compiles SQLite to wasm; set CC_wasm32_unknown_unknown to
#     use a particular one
#   - wasm-bindgen-cli at exactly the version Cargo.lock pins; set
#     WASM_BINDGEN to use one that is not first on $PATH
#
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=wasm32-unknown-unknown
OUT="$REPO/app/web/pkg"

PROFILE=release
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) PROFILE=debug; shift ;;
    -h|--help) sed -n '3,21p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }

# wasm-bindgen's command-line tool and the wasm-bindgen crate compiled into the
# module must be the same version, to the patch. They share a private format —
# the crate writes a description of every export into the .wasm and the tool
# reads it back — and a mismatch is not always refused: at best the tool
# stops with a schema error, at worst it writes glue that loads and then
# fails at the first call. So the version comes from the lockfile, not from
# whatever happens to be installed.
WANT="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' \
  "$REPO/Cargo.lock")"
[ -n "$WANT" ] || die "no wasm-bindgen in Cargo.lock; is this the repository root?"

WASM_BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
install_hint="install it with:
    cargo install wasm-bindgen-cli --version $WANT --locked
or, to keep it apart from another project's copy:
    cargo install wasm-bindgen-cli --version $WANT --locked --root <dir>
    WASM_BINDGEN=<dir>/bin/wasm-bindgen $0"
command -v "$WASM_BINDGEN" >/dev/null 2>&1 \
  || die "wasm-bindgen $WANT is needed and '$WASM_BINDGEN' was not found; $install_hint"
HAVE="$("$WASM_BINDGEN" --version | awk '{ print $2 }')"
[ "$HAVE" = "$WANT" ] \
  || die "Cargo.lock pins wasm-bindgen $WANT but $(command -v "$WASM_BINDGEN") is $HAVE; $install_hint"

rustup target list --installed 2>/dev/null | grep -qx "$TARGET" \
  || die "the $TARGET target is not installed; rustup target add $TARGET"

# SQLite is C. The `cc` crate reaches for plain `clang` when compiling for
# wasm, because gcc has no wasm backend; say so here rather than let it fail
# forty lines into a build script's output.
CLANG="${CC_wasm32_unknown_unknown:-clang}"
command -v "$CLANG" >/dev/null 2>&1 \
  || die "'$CLANG' was not found; SQLite needs a clang with the WebAssembly backend (or set CC_wasm32_unknown_unknown)"

cargo_profile=()
[ "$PROFILE" = release ] && cargo_profile=(--release)

echo "==> building accounting_bridge for $TARGET ($PROFILE)"
cargo build --manifest-path "$REPO/Cargo.toml" -p accounting_bridge \
  --target "$TARGET" "${cargo_profile[@]}"

WASM="$REPO/target/$TARGET/$PROFILE/accounting_bridge.wasm"
[ -f "$WASM" ] || die "cargo finished but $WASM is not there"

# `no-modules`, not `web`: the page loads the glue with a plain <script> that
# defines a global `wasm_bindgen`, which is the name flutter_rust_bridge's
# Dart side calls into. Emptied first, so a file the new build no longer
# produces cannot be served beside the ones it does.
echo "==> generating JavaScript bindings into ${OUT#"$REPO"/}"
rm -rf "$OUT"
"$WASM_BINDGEN" "$WASM" --out-dir "$OUT" --target no-modules --no-typescript

for f in accounting_bridge.js accounting_bridge_bg.wasm; do
  [ -f "$OUT/$f" ] || die "wasm-bindgen did not write $f"
done
echo "==> done: $(du -h "$OUT/accounting_bridge_bg.wasm" | cut -f1) of wasm"
