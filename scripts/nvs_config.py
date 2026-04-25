#!/usr/bin/env python3
"""Read NVS, merge in loraudio config keys, and optionally flash back.

Usage:
    # Generate from local backup, save to file:
    python3 nvs_config.py -i ~/phy/board1_e6fc.bin -o ~/phy/board1_e6fc_configured.bin repeater=0

    # Read from board, generate and flash in one step:
    python3 nvs_config.py -p /dev/ttyACM0 --flash repeater=0

    # Flash a previously generated image:
    espflash write-bin -p /dev/ttyACM0 0x9000 ~/phy/board1_e6fc_configured.bin
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile

# Paths relative to project
PROJ = os.path.dirname(os.path.abspath(__file__))
ESP_IDF = os.path.join(
    PROJ, ".embuild/espressif/esp-idf/v5.5.3"
)
NVS_TOOL = os.path.join(
    ESP_IDF, "components/nvs_flash/nvs_partition_tool/nvs_tool.py"
)
NVS_GEN = os.path.join(
    PROJ,
    ".embuild/espressif/python_env/idf5.5_py3.13_env/lib/python3.13/"
    "site-packages/esp_idf_nvs_partition_gen/nvs_partition_gen.py",
)

NVS_OFFSET = 0x9000
NVS_SIZE = 0x6000
NAMESPACE = "loraudio"

# Map nvs_tool encoding names to nvs_partition_gen CSV types
ENCODING_MAP = {
    "uint8_t": "u8",
    "int8_t": "i8",
    "uint16_t": "u16",
    "int16_t": "i16",
    "uint32_t": "u32",
    "int32_t": "i32",
    "uint64_t": "u64",
    "int64_t": "i64",
    "blob_data": "base64",
    "string": "string",
}

# Our config keys: name -> (csv_type, description)
CONFIG_KEYS = {
    "repeater": ("u8", "1=repeater, 0=endpoint"),
}


def read_nvs_from_port(port, tmpdir):
    """Read NVS partition from board, return path to dump."""
    dump_bin = os.path.join(tmpdir, "nvs.bin")
    subprocess.run(
        [
            "espflash", "read-flash", "-p", port,
            hex(NVS_OFFSET), hex(NVS_SIZE), dump_bin,
        ],
        check=True,
    )
    return dump_bin


def parse_nvs(bin_path):
    """Parse NVS binary into JSON entries."""
    result = subprocess.run(
        ["python3", NVS_TOOL, "-d", "minimal", "-f", "json", bin_path],
        capture_output=True, text=True, check=True,
    )
    return json.loads(result.stdout)


def entries_to_csv(entries, our_keys, tmpdir):
    """Convert JSON entries + our keys to nvs_partition_gen CSV format."""
    # Group entries by namespace to keep all keys under their namespace header
    from collections import OrderedDict
    ns_groups = OrderedDict()
    for entry in entries:
        ns = entry["namespace"]
        ns_groups.setdefault(ns, []).append(entry)

    lines = ["key,type,encoding,value"]

    for ns, ns_entries in ns_groups.items():
        lines.append(f"{ns},namespace,,")
        for entry in ns_entries:
            key = entry["key"]
            enc = entry["encoding"]
            csv_enc = ENCODING_MAP.get(enc)
            if csv_enc is None:
                print(f"WARNING: skipping unsupported encoding {enc} for {ns}:{key}")
                continue
            if csv_enc == "base64":
                raw = base64.b64decode(entry["data"])
                blob_file = os.path.join(tmpdir, f"{ns}_{key}.bin")
                with open(blob_file, "wb") as f:
                    f.write(raw)
                lines.append(f"{key},file,binary,{blob_file}")
            else:
                lines.append(f"{key},data,{csv_enc},{entry['data']}")

    # Add our namespace and keys
    if NAMESPACE not in ns_groups:
        lines.append(f"{NAMESPACE},namespace,,")
    for key, value in our_keys.items():
        csv_type = CONFIG_KEYS[key][0]
        lines.append(f"{key},data,{csv_type},{value}")

    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description="Configure loraudio NVS")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("-p", "--port", help="Serial port to read NVS from")
    source.add_argument("-i", "--input", help="Local NVS binary to read from")
    parser.add_argument("-o", "--output",
                        help="Write generated NVS image here (no flash)")
    parser.add_argument("--flash", action="store_true",
                        help="Flash to board (requires -p)")
    parser.add_argument("settings", nargs="+",
                        help="key=value pairs, e.g. repeater=0")
    args = parser.parse_args()

    if args.flash and not args.port:
        print("ERROR: --flash requires -p")
        sys.exit(1)

    # Parse key=value pairs
    our_keys = {}
    for s in args.settings:
        if "=" not in s:
            print(f"ERROR: expected key=value, got: {s}")
            sys.exit(1)
        k, v = s.split("=", 1)
        if k not in CONFIG_KEYS:
            print(f"ERROR: unknown key '{k}'. Known: {list(CONFIG_KEYS.keys())}")
            sys.exit(1)
        our_keys[k] = v

    with tempfile.TemporaryDirectory() as tmpdir:
        if args.input:
            print(f"Reading NVS from {args.input}...")
            entries = parse_nvs(args.input)
        else:
            print(f"Reading NVS from {args.port}...")
            dump = read_nvs_from_port(args.port, tmpdir)
            entries = parse_nvs(dump)
        print(f"  Found {len(entries)} existing entries")

        csv_content = entries_to_csv(entries, our_keys, tmpdir)

        # Generate NVS binary
        csv_file = os.path.join(tmpdir, "merged.csv")
        bin_file = os.path.join(tmpdir, "merged.bin")
        with open(csv_file, "w") as f:
            f.write(csv_content)

        subprocess.run(
            ["python3", NVS_GEN, "generate", csv_file, bin_file, hex(NVS_SIZE)],
            check=True,
        )
        print(f"Generated {os.path.getsize(bin_file)} byte NVS image")

        if args.output:
            import shutil
            shutil.copy2(bin_file, args.output)
            print(f"Saved to {args.output}")

        if args.flash:
            print(f"Writing NVS to {args.port} at {hex(NVS_OFFSET)}...")
            subprocess.run(
                [
                    "espflash", "write-bin", "-p", args.port,
                    hex(NVS_OFFSET), bin_file,
                ],
                check=True,
            )
            print("Done! Reset the board to pick up new config.")

        if not args.output and not args.flash:
            print("(No -o or --flash specified, image not saved)")


if __name__ == "__main__":
    main()
