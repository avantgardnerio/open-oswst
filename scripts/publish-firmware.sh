#!/usr/bin/env bash
# Publish an update bundle to the management server (server/): build the
# firmware, save its app image, and pack it with the web app into
# DIR/bundle.tar.gz (core/src/bundle.rs):
#
#   firmware.bin   -> the radio's spare app slot
#   www/...        -> the radio's storage, /data/www/...
#
# Radios are offered it the next time someone asks: the menu's Update, or
# POST /api/management/update. The server notices the new file by itself.
#
# Renamed into place, never written there: a radio part way through a
# download is told "Image changed" rather than sent half of two bundles.
#
# Usage: scripts/publish-firmware.sh [DIR]   (default /srv/oswst)

set -e

DIR="${1:-/srv/oswst}"
cd "$(dirname "$0")/.."
. ~/export-esp.sh

cargo build

# The bundle's contents, laid out as they land on the radio
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
espflash save-image --chip esp32s3 target/xtensa-esp32s3-espidf/debug/open-oswst "$STAGE/firmware.bin"
cp -r www "$STAGE/www"

# ustar: the radio's tar reader reads only that. Firmware first, then files
NEW="$DIR/.bundle.tar.gz.new"
tar --format=ustar -C "$STAGE" -cf - firmware.bin www | gzip -9 > "$NEW"
mv "$NEW" "$DIR/bundle.tar.gz"
echo "Published $(git describe --always --dirty) to $DIR/bundle.tar.gz ($(stat -c %s "$DIR/bundle.tar.gz") B)"
