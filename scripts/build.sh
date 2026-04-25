#!/usr/bin/env bash
# Build and flash all connected boards.
# Usage: ./build.sh [--flash]

set -e

. ~/export-esp.sh

cargo build

if [[ "$1" == "--flash" ]]; then
    "$(dirname "$0")/flash-all.sh"
fi
