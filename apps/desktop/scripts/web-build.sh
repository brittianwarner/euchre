#!/usr/bin/env bash
# Build the Rune web app (the euchre-desktop crate on wasm32) into a static tree.
#
#   apps/desktop/scripts/web-build.sh --out DIR
#
# Output (docs/port/10-web-rune-plan.md section 2, amended by 12):
#   DIR/rune.html                                  the loader, filled (no-cache shell)
#   DIR/_rune/immutable/<hash>/app.js, app_bg.wasm, snippets/**, Inter-*.ttf
#   DIR/_rune/immutable/art-<arthash>/cards/*.png, euchre-hero.webp
# <hash> = first 16 hex of sha256 over app_bg.wasm, app.js and the snippets (W-9);
# <arthash> over the art files (W-8). Needs apps/desktop/js/rivet-bridge.js first:
#   bun run --cwd apps/desktop bridge
#
# Env: RUNE_FEATURES (cargo features, default none), CARGO_TARGET_DIR.
set -euo pipefail

out=
while (($#)); do
  case "$1" in
    --out) out=${2:?--out needs a directory}; shift 2 ;;
    *) echo "usage: web-build.sh --out DIR" >&2; exit 2 ;;
  esac
done
[[ -n $out ]] || { echo "usage: web-build.sh --out DIR" >&2; exit 2; }

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
cd "$root"
mkdir -p "$out"
out=$(cd "$out" && pwd)

[[ -f apps/desktop/js/rivet-bridge.js ]] || {
  echo "apps/desktop/js/rivet-bridge.js is missing: run 'bun run --cwd apps/desktop bridge' first." >&2
  exit 1
}

# The wasm-bindgen CLI must be the version the lockfile pins.
version=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, "", $0); print; exit }' Cargo.lock)
if ! command -v wasm-bindgen >/dev/null || [[ $(wasm-bindgen --version) != "wasm-bindgen $version" ]]; then
  echo "Install the matching CLI: cargo install wasm-bindgen-cli --version $version --locked" >&2
  exit 2
fi

# Rune's release profile for the web (size first), and no builder paths in the wasm.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=${CARGO_PROFILE_RELEASE_OPT_LEVEL:-s} \
  CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO:-thin} \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  CARGO_PROFILE_RELEASE_DEBUG=0 \
  CARGO_PROFILE_RELEASE_STRIP=debuginfo
# (Skipped when the caller already set them, as the Dockerfile does so the dependency layer matches.)
case "${RUSTFLAGS:-}" in
  *remap-path-prefix*) ;;
  *) export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/home --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo" ;;
esac

features=${RUNE_FEATURES-}
feature_args=()
[[ -z $features ]] || feature_args=(--features "$features")
cargo build --release --locked --target wasm32-unknown-unknown -p euchre-desktop --lib ${feature_args[@]+"${feature_args[@]}"}

target=${CARGO_TARGET_DIR:-$root/target}
artifact=$target/wasm32-unknown-unknown/release/euchre_desktop.wasm
[[ -f $artifact ]] || { echo "missing $artifact" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
wasm-bindgen --target web --out-dir "$stage/app" --out-name app --remove-name-section "$artifact"
rm -f "$stage"/app/*.d.ts

# Rune's loader fetches the full Inter faces beside app.js after the first frame. The dep-info
# lists only path packages, so look in the cargo checkout of the pinned rune revision.
rev=$(sed -n 's|^source = "git+https://github.com/anomalyco/rune?rev=\([0-9a-f]*\)#.*|\1|p' Cargo.lock | head -1)
inter=
for dir in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/rune-*/"${rev:0:7}"/crates/rune/assets/web; do
  [[ -f $dir/Inter-Regular.ttf ]] && inter=$dir && break
done
[[ -n $inter ]] || { echo "Inter faces for rune $rev not found in the cargo git checkouts" >&2; exit 1; }
cp "$inter"/Inter-*.ttf "$stage/app/"

# Art by absolute URL (W-8): the Svelte site's card faces and hero, with the desktop Ace on top
# (D31 S-2: the two builds draw a different Ace of spades).
mkdir -p "$stage/art/cards"
cp apps/web/static/art/cards/*.png "$stage/art/cards/"
cp apps/web/static/art/euchre-hero.webp "$stage/art/"
cp apps/desktop/assets/cards/AS.png "$stage/art/cards/AS.png"

sha() { if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; }
hash_files() { (cd "$1" && LC_ALL=C find . -type f | LC_ALL=C sort | while read -r f; do printf '%s\n' "$f"; cat "$f"; done) | sha | cut -c1-16; }
hash=$( (cat "$stage/app/app_bg.wasm" "$stage/app/app.js"; hash_files "$stage/app/snippets") | sha | cut -c1-16)
arthash=$(hash_files "$stage/art")

dest=$out/_rune/immutable
rm -rf "$dest/$hash" "$dest/art-$arthash"
mkdir -p "$dest"
mv "$stage/app" "$dest/$hash"
mv "$stage/art" "$dest/art-$arthash"

sed -e "s|__RUNE_HASH__|$hash|g" -e "s|__ART_BASE__|/_rune/immutable/art-$arthash/|g" \
  apps/desktop/web/rune.html > "$out/rune.html"
if grep -q '__RUNE_HASH__\|__ART_BASE__' "$out/rune.html"; then
  echo "rune.html still has a placeholder" >&2
  exit 1
fi
echo "Rune web build: $out (hash $hash, art $arthash)"
