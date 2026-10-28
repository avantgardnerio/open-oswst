"""Continuity sweep on the Jumperless: which of the given breadboard rows connect to which.

  jumperless_sweep.py <session> <name> --rows 31-33,36-38,51-60 [--record]

One row at a time gets the bottom rail at 1 V; ADC0 visits every other row; above 0.5 V
counts as connected. Every pair is tested from both ends, so the result checks itself.
Before anything is driven, every row is read idle: a row already above 0.3 V means something
live is touching it (floating long wires pick up ~0.15 V; a real fault read 4.7 V), and the
sweep stops.

The sweep draws itself on the other half of the breadboard, on each test row's twin column
(row 31 -> row 1): red = driven, blue = probed, and a connection found leaves both twins lit
in one shared colour. --record films it into the session folder (bench_record.py).

Writes the pairs to the session log and prints them. With nothing to compare against, one
sweep is a netlist; two sweeps (e.g. plugs out / plugs in) diff into a pin map.
"""
import argparse
import pathlib
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import bench  # noqa: E402

DEVICE_CODE = """
import time

ROWS = {rows}
SHOW = {show}
DRIVE_VOLTS = 1.0
CONNECTED_VOLTS = 0.5
IDLE_LIMIT_VOLTS = 0.3
DRIVE_COLOR = 0x400000
PROBE_COLOR = 0x000040
TESTED_COLOR = 0x040404
PAIR_COLORS = [0x303000, 0x003020, 0x300030, 0x002830, 0x301000, 0x103000, 0x200830, 0x302010]
OFF = 0x000000


def twin(row):
    # Overlay grid: x = column 1-30, y 1-5 = top half (rows 1-30), 6-10 = bottom (31-60).
    # A bottom row draws on the top half and the other way round, on that half's first
    # 3 holes (A-C or F-H).
    if row > 30:
        return row - 30, 1
    return row, 6


def mark(row, line, color):
    if SHOW:
        column, first_line = twin(row)
        overlay_set_pixel(column, first_line + line, color)


def read_row(row):
    connect(ADC0, row)
    time.sleep(0.03)
    volts = sum(adc_get(0) for _ in range(4)) / 4
    disconnect(ADC0, row)
    return volts


overlay_clear_all()
dac_set(BOTTOM_RAIL, 0.0, False)
for row in ROWS:
    mark(row, 0, TESTED_COLOR)

idle = [(row, read_row(row)) for row in ROWS]
print("IDLE", " ".join("%d:%.3f" % pair for pair in idle))
driven = [row for row, volts in idle if abs(volts) > IDLE_LIMIT_VOLTS]
if driven:
    print("ABORT rows already driven:", driven)
else:
    pair_color = {{}}
    row_color = {{}}
    dac_set(BOTTOM_RAIL, DRIVE_VOLTS, False)
    for drive in ROWS:
        connect(BOTTOM_RAIL, drive)
        mark(drive, 0, DRIVE_COLOR)
        time.sleep(0.05)
        hits = []
        for row in ROWS:
            if row == drive:
                continue
            mark(row, 1, PROBE_COLOR)
            volts = read_row(row)
            # Put back whatever the probe mark covered: a found pair's colour or nothing.
            mark(row, 1, row_color.get(row, OFF))
            if volts > CONNECTED_VOLTS:
                hits.append("%d:%.3f" % (row, volts))
                key = (min(drive, row), max(drive, row))
                if key not in pair_color:
                    pair_color[key] = PAIR_COLORS[len(pair_color) % len(PAIR_COLORS)]
                for end in key:
                    row_color[end] = pair_color[key]
                    mark(end, 1, pair_color[key])
                    mark(end, 2, pair_color[key])
        print("DRIVE", drive, "->", " ".join(hits))
        disconnect(BOTTOM_RAIL, drive)
        mark(drive, 0, TESTED_COLOR)
    dac_set(BOTTOM_RAIL, 0.0, False)
print("DONE rail", dac_get(BOTTOM_RAIL))
"""


def parse_rows(spec):
    rows = []
    for part in spec.split(","):
        first, _, last = part.partition("-")
        rows.extend(range(int(first), int(last or first) + 1))
    return rows


def parse_pairs(stdout):
    # "DRIVE 31 -> 53:0.970 54:0.968" -> {(31, 53), (31, 54)}, each pair as (driven, seen).
    seen = set()
    for line in stdout.splitlines():
        if not line.startswith("DRIVE"):
            continue
        drive_part, _, hits_part = line[len("DRIVE"):].partition("->")
        drive = int(drive_part)
        for hit in hits_part.split():
            seen.add((drive, int(hit.split(":")[0])))
    return seen


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("session")
    parser.add_argument("name", help="name for this sweep in the log and the video file")
    parser.add_argument("--rows", required=True, help="e.g. 31-33,36-38,51-60")
    parser.add_argument("--record", action="store_true", help="film the sweep into the session folder")
    args = parser.parse_args()

    rows = parse_rows(args.rows)
    # The display needs every twin column free: a twin that is itself a test row would
    # paint over a part under test.
    show = not any((row - 30 if row > 30 else row + 30) in rows for row in rows)
    if not show:
        print("rows on both halves overlap their twins: sweeping without the on-board display")

    session_dir = bench.session_dir(args.session)
    recorder = None
    stop_file = session_dir / ".stop-recording"
    if args.record:
        video = session_dir / f"{args.name}.mp4"
        recorder = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).parent / "bench_record.py"),
                                     str(video), str(stop_file), "600"])
        # The recorder needs ~2 s to settle exposure and time its frame rate.
        time.sleep(4)

    bench.log(args.session, f"Sweep **{args.name}**: rows {args.rows}" + (f", video {args.name}.mp4" if args.record else ""))
    # ~0.16 s per read and rows^2 reads, plus margin.
    timeout_s = 30 + 0.2 * len(rows) ** 2
    try:
        stdout, stderr = bench.run(DEVICE_CODE.format(rows=rows, show=show), timeout_s)
    finally:
        if recorder:
            time.sleep(3)
            stop_file.touch()
            recorder.wait()
            stop_file.unlink(missing_ok=True)
    print(stdout, end="")
    if stderr:
        print("STDERR:", stderr)

    seen = parse_pairs(stdout)
    pairs = sorted({(min(pair), max(pair)) for pair in seen})
    one_sided = sorted(pair for pair in seen if (pair[1], pair[0]) not in seen)
    summary = "Pairs: " + (", ".join(f"{low}-{high}" for low, high in pairs) or "none")
    if one_sided:
        summary += "\nSEEN FROM ONE END ONLY (suspect): " + ", ".join(f"{drive}->{row}" for drive, row in one_sided)
    print(summary)
    bench.log(args.session, f"```\n{stdout}{stderr}```\n\n{summary}")


if __name__ == "__main__":
    main()
