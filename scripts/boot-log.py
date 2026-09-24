#!/usr/bin/env python3
"""Reset an ESP32-S3 and capture its serial log from the very first line.

A plain `cat` of the port misses boot output: by the time it attaches, the
board has already booted. This resets via the RTS line and reads in the same
process, so nothing is lost and nothing else collides with the port.

Don't run while espflash (or another reader) holds the port.

Usage:
    python3 scripts/boot-log.py /dev/ttyACM1        # 8 seconds
    python3 scripts/boot-log.py /dev/ttyACM1 20     # 20 seconds

Needs pyserial: `sudo apt install python3-serial` or `pip install pyserial`.
"""

import sys
import time

import serial


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    port = sys.argv[1]
    secs = float(sys.argv[2]) if len(sys.argv) > 2 else 8.0

    # Hard reset: pulse RTS (EN) with DTR (IO0) released, so it boots the app
    s = serial.Serial(port, 115200, timeout=0.2)
    s.dtr = False
    s.rts = True
    time.sleep(0.1)
    s.rts = False
    s.close()

    # USB-Serial-JTAG re-enumerates on reset; reopen as soon as it's back
    end = time.time() + secs
    s = None
    while s is None and time.time() < end:
        try:
            s = serial.Serial(port, 115200, timeout=0.2)
        except serial.SerialException:
            time.sleep(0.05)
    if s is None:
        sys.exit(f"{port} didn't come back after reset")

    while time.time() < end:
        sys.stdout.write(s.read(4096).decode(errors="replace"))
        sys.stdout.flush()


if __name__ == "__main__":
    main()
