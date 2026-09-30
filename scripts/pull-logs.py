#!/usr/bin/env python3
"""Copy the log files off a board's /data partition.

Reads the `storage` partition over USB with esptool (~70s for the whole
11.6MB; espflash reads at ~11KB/s, which would take ~18min), then unpacks its
LittleFS filesystem on the laptop. The board never shows up as a disk: its
USB port is a serial port.

Before reading, listens briefly for a live log line to pair the board's
uptime with the laptop's clock (written to anchor.txt). That's what lines log
timestamps up with a phone GPS track. Reading resets the board, so this is
the last chance to catch that board's uptime.

Don't run while another reader holds the port.

The raw image is kept as storage.bin next to the files, so a failed unpack
can be retried with --image without reading the board again.

Usage:
    .venv/bin/python scripts/pull-logs.py /dev/ttyACM1            # into logs/<MAC>-<time>/
    .venv/bin/python scripts/pull-logs.py /dev/ttyACM1 some/dir
    .venv/bin/python scripts/pull-logs.py --image logs/X/storage.bin   # unpack again, no board

Needs esptool, littlefs-python and pyserial (in requirements.txt), and
espflash on PATH (for the MAC).
"""

import csv
import datetime
import re
import subprocess
import sys
import time
from pathlib import Path

import serial
from littlefs import LittleFS

REPO = Path(__file__).resolve().parent.parent

# Must match the firmware's CONFIG_LITTLEFS_* settings (see sdkconfig). name_max
# is what the superblock on the board records (255), not CONFIG_LITTLEFS_OBJ_NAME_LEN
BLOCK_SIZE = 4096
LFS_OPTIONS = dict(read_size=128, prog_size=128, lookahead_size=128, cache_size=512, name_max=255)

LIVE_LINE = re.compile(rb"^[EWIDV] \((\d+)\) ")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if sys.argv[1] == "--image":
        image = Path(sys.argv[2])
        print(f"{extract(image.read_bytes(), image.parent)} files -> {image.parent}")
        return
    port = sys.argv[1]

    offset, size = storage_partition()
    anchor = catch_uptime(port)
    mac = board_mac(port)

    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "logs" / f"{mac}-{stamp}"
    out.mkdir(parents=True, exist_ok=True)

    image = out / "storage.bin"
    print(f"Reading {size // 1024} KB from 0x{offset:x} (~80s)...")
    subprocess.run(
        [sys.executable, "-m", "esptool", "--chip", "esp32s3", "-p", port,
         "read-flash", hex(offset), hex(size), str(image)],
        check=True,
    )

    if anchor:
        uptime_ms, wall = anchor
        (out / "anchor.txt").write_text(f"uptime_ms={uptime_ms}\nwall={wall.isoformat()}\n")
    count = extract(image.read_bytes(), out)
    print(f"{count} files -> {out}")
    if not anchor:
        print("No live log line caught, so no clock anchor (anchor.txt not written)")


def storage_partition():
    """(offset, size) of the `storage` partition, from partitions.csv."""
    with open(REPO / "partitions.csv") as f:
        rows = csv.reader(line for line in f if not line.lstrip().startswith("#"))
        for row in rows:
            if row and row[0].strip() == "storage":
                return int(row[3], 0), int(row[4], 0)
    sys.exit("No `storage` partition in partitions.csv")


def catch_uptime(port, secs=5.0):
    """Listen for one live log line: (uptime_ms, laptop time), or None."""
    print(f"Listening {secs:.0f}s for a live log line (to anchor the clock)...")
    with serial.Serial(port, 115200, timeout=0.2) as s:
        end = time.time() + secs
        while time.time() < end:
            m = LIVE_LINE.match(s.readline())
            if m:
                return int(m.group(1)), datetime.datetime.now().astimezone()
    return None


def board_mac(port):
    """The board's MAC, for naming the output folder."""
    info = subprocess.run(
        ["espflash", "board-info", "-p", port], capture_output=True, text=True
    ).stdout
    m = re.search(r"MAC address:\s+([0-9a-f:]+)", info)
    return m.group(1).replace(":", "").upper() if m else "unknown"


def extract(image, out):
    """Copy every file in the LittleFS image into `out`. Returns the count."""
    fs = LittleFS(block_size=BLOCK_SIZE, block_count=len(image) // BLOCK_SIZE, mount=False, **LFS_OPTIONS)
    fs.context.buffer = bytearray(image)
    fs.mount()
    count = 0
    for root, _dirs, files in fs.walk("/"):
        for name in files:
            src = f"{root.rstrip('/')}/{name}"
            dest = out / src.lstrip("/")
            dest.parent.mkdir(parents=True, exist_ok=True)
            with fs.open(src, "rb") as f:
                dest.write_bytes(f.read())
            count += 1
    return count


if __name__ == "__main__":
    main()
