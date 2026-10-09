#!/usr/bin/env bash
# Lay out the web app as it goes to a radio, in DEST: www/'s own files
# (not node_modules or package files), plus the node_modules files
# index.html's import map names, at the same paths. npm ci first if
# node_modules is missing (package-lock.json pins the versions).
# Used by publish-firmware.sh (the bundle) and upload-www.sh (WebDAV).
#
# Usage: scripts/stage-www.sh DEST

set -e

DEST="${1:?usage: $0 DEST}"
cd "$(dirname "$0")/../www"

[ -d node_modules ] || npm ci --no-audit --no-fund

mkdir -p "$DEST"
for file in *; do
    case "$file" in
        node_modules | package.json | package-lock.json) ;;
        *) cp -r "$file" "$DEST/" ;;
    esac
done
for module in $(grep -oE '\./node_modules/[^"]+' index.html); do
    mkdir -p "$DEST/$(dirname "$module")"
    cp "$module" "$DEST/$module"
done
