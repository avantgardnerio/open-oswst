#!/usr/bin/env bash
# Flash all connected Espressif boards with the built binary + partition table.
# Usage: ./flash-all.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-esp.sh"

BIN="target/xtensa-esp32s3-espidf/debug/open-oswst"
PTABLE="target/xtensa-esp32s3-espidf/debug/partition-table.bin"
# Our own bootloader, not espflash's: it's built from sdkconfig.defaults and
# switches the flash to QIO
BOOTLOADER=$(ls -t target/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin 2>/dev/null | head -1)

if [[ ! -f "$BIN" || ! -f "$PTABLE" || ! -f "$BOOTLOADER" ]]; then
    echo "Build artifacts not found. Run 'cargo build' first." >&2
    exit 1
fi

if ! esp_devices; then
    echo "No Espressif serial devices found." >&2
    exit 1
fi

echo "Flashing ${#DEVS[@]} device(s): ${DEVS[*]}"

PIDS=()
for DEV in "${DEVS[@]}"; do
    # otadata erased: after an OTA update, this makes the board boot the app
    # flashed here (ota_0) instead of the other slot
    espflash flash -p "$DEV" --bootloader "$BOOTLOADER" --partition-table "$PTABLE" \
        --erase-parts otadata "$BIN" &
    PIDS+=($!)
done

FAIL=0
for i in "${!PIDS[@]}"; do
    if ! wait "${PIDS[$i]}"; then
        echo "FAILED: ${DEVS[$i]}" >&2
        FAIL=1
    fi
done

if [[ $FAIL -eq 0 ]]; then
    echo "All ${#DEVS[@]} devices flashed successfully."
else
    echo "Some devices failed to flash." >&2
    exit 1
fi
