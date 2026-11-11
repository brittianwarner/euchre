#!/usr/bin/env bash
# Merge the Svelte build and the Rune web build into the tree euchre-server serves.
#
#   apps/desktop/scripts/web-site.sh <svelte build/client> <rune out> <dest>
#
# The Svelte tree is copied as is (index.html is the homepage and the classic fallback shell);
# the Rune tree (rune.html, _rune/) goes on top and gets .br/.gz siblings.
set -euo pipefail
(($# == 3)) || { echo "usage: web-site.sh <svelte client> <rune out> <dest>" >&2; exit 2; }
svelte=$1 rune=$2 dest=$3
here=$(cd "$(dirname "$0")" && pwd)
[[ -f $svelte/index.html ]] || { echo "$svelte has no index.html" >&2; exit 1; }
[[ -f $rune/rune.html && -d $rune/_rune ]] || { echo "$rune is not a Rune web build" >&2; exit 1; }
rm -rf "$dest"
mkdir -p "$dest"
cp -R "$svelte"/. "$dest"/
cp -R "$rune"/. "$dest"/
bun "$here/precompress.ts" "$dest/rune.html" "$dest/_rune"
