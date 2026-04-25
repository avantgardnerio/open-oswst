#!/usr/bin/env bash
# Flash all connected /dev/ttyACM* boards with the built binary + partition table.
# Usage: ./flash-all.sh

set -e

BIN="target/xtensa-esp32s3-espidf/debug/loraudio"
PTABLE="target/xtensa-esp32s3-espidf/debug/partition-table.bin"

if [[ ! -f "$BIN" || ! -f "$PTABLE" ]]; then
    echo "Build artifacts not found. Run 'cargo build' first." >&2
    exit 1
fi

DEVS=(/dev/ttyACM*)
if [[ ${#DEVS[@]} -eq 0 || ! -e "${DEVS[0]}" ]]; then
    echo "No /dev/ttyACM* devices found." >&2
    exit 1
fi

echo "Flashing ${#DEVS[@]} device(s): ${DEVS[*]}"

PIDS=()
for DEV in "${DEVS[@]}"; do
    espflash flash -p "$DEV" --partition-table "$PTABLE" "$BIN" &
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
