#!/usr/bin/env bash
# Monitor multiple serial radios in one merged, color-coded log.
# Usage: ./monitor-all.sh [baud]
#   Default baud: 115200
#   Auto-detects /dev/ttyACM* devices present at launch.

BAUD="${1:-115200}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOGFILE="$SCRIPT_DIR/../out/monitor.log"
mkdir -p "$(dirname "$LOGFILE")"
COLORS=("\e[32m" "\e[33m" "\e[36m" "\e[35m" "\e[34m")  # green yellow cyan magenta blue
RESET="\e[0m"
PIDS=()

cleanup() {
    kill "${PIDS[@]}" 2>/dev/null
    wait 2>/dev/null
    echo -e "\n${RESET}All monitors stopped."
}
trap cleanup EXIT INT TERM

DEVS=(/dev/ttyACM*)
if [[ ${#DEVS[@]} -eq 0 || ! -e "${DEVS[0]}" ]]; then
    echo "No /dev/ttyACM* devices found." >&2
    exit 1
fi

echo "Monitoring ${#DEVS[@]} device(s) at ${BAUD} baud: ${DEVS[*]}"
echo "Logging to $LOGFILE"
echo "Press Ctrl-C to stop."
echo "---"

for i in "${!DEVS[@]}"; do
    DEV="${DEVS[$i]}"
    COLOR="${COLORS[$((i % ${#COLORS[@]}))]}"
    TAG=$(basename "$DEV")

    # Configure serial port (raw mode, no echo, set baud)
    stty -F "$DEV" "$BAUD" raw -echo -hupcl 2>/dev/null

    # Read and prefix each line with timestamp + device tag
    while IFS= read -r line; do
        TS=$(date +%H:%M:%S.%3N)
        PLAIN=$(printf "[%s %s] %s" "$TS" "$TAG" "$line")
        printf "${COLOR}%s${RESET}\n" "$PLAIN"
        printf "%s\n" "$PLAIN" >> "$LOGFILE"
    done < "$DEV" &
    PIDS+=($!)
done

wait
