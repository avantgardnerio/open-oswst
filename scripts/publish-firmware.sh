#!/usr/bin/env bash
# Publish the firmware to the management server (server/): build it, save
# the app image, and rename it over DIR/firmware.bin. Radios are offered it
# the next time someone asks: the menu's Update, or
# POST /api/management/update. The server notices the new file by itself.
#
# Renamed into place, never written there: a radio part way through a
# download is told "Image changed" rather than sent half of two images.
#
# Usage: scripts/publish-firmware.sh [DIR]   (default /srv/oswst)

set -e

DIR="${1:-/srv/oswst}"
cd "$(dirname "$0")/.."
. ~/export-esp.sh

cargo build
NEW="$DIR/.firmware.bin.new"
espflash save-image --chip esp32s3 target/xtensa-esp32s3-espidf/debug/open-oswst "$NEW"
mv "$NEW" "$DIR/firmware.bin"
echo "Published $(git describe --always --dirty) to $DIR/firmware.bin"
