#!/usr/bin/env python3
"""Set boards up over USB: the firmware, and a fresh storage partition holding
that board's /data/config.toml (its mode, any other settings and flags its
entry gives, and the WiFi networks to join).

Each board's settings come from boards.toml in the repo root, kept out of git
because it holds WiFi passwords (copy boards.example.toml to start). Every
network under [all] goes on every board, first; a board's own come after:

    [[all.wifi]]
    ssid = "Starlink"
    password = "..."

    ["A4:CB:8F:A2:0F:5C"]
    mode = "repeater"

    ["F8:5B:1B:A2:C6:2C"]
    mode = "normal"
    name = "handheld"
    tx_power_dbm = 6          # any setting in core/src/config.rs, copied as-is

    ["F8:5B:1B:A2:C6:2C".flags]
    send_position = true      # and its [flags]

A board missing from boards.toml gets mode "normal" and the [all] networks.

🚨 This ERASES the board's storage: logs and all. Pull them first
(scripts/pull-logs.py). Also needed whenever partitions.csv changes.

Usage:
    .venv/bin/python scripts/provision.py                      # every connected board
    .venv/bin/python scripts/provision.py A4:CB:8F:A2:0F:5C    # just these

Build first (cargo build). Needs littlefs-python and pyserial
(requirements.txt), and espflash on the PATH.
"""

import csv
import glob
import json
import subprocess
import sys
import tomllib
from pathlib import Path

import serial.tools.list_ports
from littlefs import LittleFS

REPO = Path(__file__).resolve().parent.parent
BUILD = REPO / "target/xtensa-esp32s3-espidf/debug"
ESP_VID = 0x303A
# Must match the firmware's LittleFS settings (same as pull-logs.py)
BLOCK_SIZE = 4096
LFS_OPTIONS = dict(read_size=128, prog_size=128, lookahead_size=128, cache_size=512, name_max=255)
MODES = ("normal", "repeater", "echo")


def main():
    boards = tomllib.loads((REPO / "boards.toml").read_text())
    wanted = set(sys.argv[1:])
    ports = [p for p in serial.tools.list_ports.comports()
             if p.vid == ESP_VID and (not wanted or p.serial_number in wanted)]
    if not ports:
        sys.exit("No matching boards connected")
    offset, size = partition("storage")
    for port in ports:
        mac = port.serial_number
        config = config_toml(boards, mac)
        print(f"== {mac} on {port.device}\n{indent(masked(config))}")
        image = storage_image(config, size)
        flash(port.device, offset, image)
    print("Done.")


def config_toml(boards, mac):
    """This board's config.toml text. Strings are written JSON-quoted, which
    TOML reads the same."""
    board = boards.get(mac, {})
    mode = board.get("mode", "normal")
    if mode not in MODES:
        sys.exit(f"{mac}: mode {mode!r} isn't one of {MODES}")
    networks = boards.get("all", {}).get("wifi", []) + board.get("wifi", [])
    lines = [f"mode = {json.dumps(mode)}"]
    # Any other setting the entry gives (name, tx_power_dbm...), as-is: the
    # firmware checks them. JSON writes strings, numbers and true/false the
    # way TOML reads them
    for key, value in board.items():
        if key not in ("mode", "wifi", "flags"):
            lines.append(f"{key} = {json.dumps(value)}")
    flags = board.get("flags", {})
    if flags:
        lines += ["", "[flags]"]
        lines += [f"{key} = {json.dumps(value)}" for key, value in flags.items()]
    for net in networks:
        lines += ["", "[[wifi]]",
                  f"ssid = {json.dumps(net['ssid'])}",
                  f"password = {json.dumps(net['password'])}"]
    return "\n".join(lines) + "\n"


def storage_image(config, size):
    """A whole storage partition: LittleFS holding just config.toml."""
    fs = LittleFS(block_size=BLOCK_SIZE, block_count=size // BLOCK_SIZE, **LFS_OPTIONS)
    with fs.open("config.toml", "w") as f:
        f.write(config)
    return bytes(fs.context.buffer)


def flash(device, offset, image):
    image_path = BUILD / "storage.bin"
    image_path.write_bytes(image)
    bootloader = glob.glob(str(BUILD / "build/esp-idf-sys-*/out/build/bootloader/bootloader.bin"))[0]
    # Our bootloader (QIO, see flash-all.sh); otadata erased so it boots ota_0
    run(["espflash", "flash", "-p", device, "--bootloader", bootloader,
         "--partition-table", str(BUILD / "partition-table.bin"),
         "--erase-parts", "otadata", str(BUILD / "open-oswst")])
    run(["espflash", "write-bin", "-p", device, hex(offset), str(image_path)])


def run(cmd):
    print("  $", " ".join(cmd))
    subprocess.run(cmd, check=True)


def partition(name):
    """(offset, size) of a partition, from partitions.csv."""
    with open(REPO / "partitions.csv") as f:
        for row in csv.reader(line for line in f if not line.lstrip().startswith("#")):
            if row and row[0].strip() == name:
                return int(row[3], 0), int(row[4], 0)
    sys.exit(f"No {name} partition in partitions.csv")


def masked(config):
    """The config for the terminal, passwords hidden."""
    return "\n".join('password = "***"' if line.startswith("password") else line
                     for line in config.splitlines())


def indent(text):
    return "\n".join("    " + line for line in text.splitlines())


if __name__ == "__main__":
    main()
