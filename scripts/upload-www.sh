#!/usr/bin/env bash
# Upload the web app (www/, as scripts/stage-www.sh lays it out) to a
# radio's storage over WebDAV, at /fs/www/: for trying a change without
# publishing a bundle. Then browse to http://<radio>/
#
# Usage: scripts/upload-www.sh <radio address or name>   e.g. 192.168.0.219

set -e

RADIO="${1:?usage: $0 <radio address or name>}"
cd "$(dirname "$0")/.."

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
scripts/stage-www.sh "$STAGE"
cd "$STAGE"

# Folders first, each before what's in it. One already there answers
# MKCOL with 405, which is fine
for folder in $(find . -type d | sort); do
    curl -s -o /dev/null -X MKCOL "http://$RADIO/fs/www/${folder#./}"
done
for file in $(find . -type f | sort); do
    curl -sf -T "$file" "http://$RADIO/fs/www/${file#./}" > /dev/null
    echo "uploaded ${file#./}"
done
echo "Open http://$RADIO/"
