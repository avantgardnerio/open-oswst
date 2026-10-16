#!/usr/bin/env python3
"""Drive the Genmitsu 3018 (GRBL) from the command line: home it, set the work
origin on a PCB blank, probe the blank's height, and stream a job with every
cut bent to follow that height.

Why the height map: the copper is 35 um thick and a taped-down sheet bows by
more than that, so a V-bit set to cut 0.05 mm deep at one corner cuts air (or
snaps) at another. GRBL can't correct for it, and pcb2gcode's own autoleveller
needs LinuxCNC/Mach variables GRBL doesn't have. So we probe a grid with the
bit itself (G38.2: the bit closes the probe circuit when it touches the
copper) and add the measured height to every Z before sending the job.

Setup: probe clip on the bit; the probe puck lies metal face down on the
copper, so the whole copper sheet is the probe plate (thickness 0).

    cnc.py status
    cnc.py home
    cnc.py jog DX DY [DZ]          move by mm, e.g. to above the blank's corner
    cnc.py zero                    tap test, then here is work X0 Y0; probe down
                                   (from up to 45 mm): copper = Z0
    cnc.py probe W H map.json      probe a W x H mm grid from the origin
    cnc.py run job.nc map.json     stream the job, Z bent by the height map

Ctrl-C during a run: feed hold, then a GRBL reset. The spindle stops and the
position is lost: home again.
"""

import argparse
import collections
import json
import math
import re
import sys
import time

import serial

# The 3018's CH340. Never the bare vendor id: the Z-Wave dongle (ttyACM0) is
# the same vendor
PORT = "/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0"
# Bytes GRBL can queue unanswered. This ARM build reports 254 ([OPT:VZL,35,254]);
# the classic 127 is plenty for short lines
RX_BUFFER = 127
SAFE_Z = 3.0  # mm above the copper for travel to and from the job
PROBE_CLEARANCE = 1.0  # mm above the copper between grid points
PROBE_DEPTH = -1.5  # mm: the deepest a grid probe looks before GRBL alarms
PROBE_FEED = 50  # mm/min: slow, so the bit stops before it digs in
# `zero` finds the copper in two touches: a fast search down most of the Z
# travel (the bit may start 30+ mm up), then back off and touch again slowly,
# because the fast touch overshoots a little before GRBL stops
ZERO_SEARCH = 45.0  # mm
ZERO_FAST_FEED = 150  # mm/min
ZERO_SLOW_FEED = 20  # mm/min
ZERO_BACK_OFF = 1.0  # mm
TAP_WAIT = 60  # s for the user to tap the puck on the bit before `zero` moves
JOG_FEED = 600  # mm/min
SEGMENT = 1.0  # mm: cuts are split so Z can follow the map between points


class Grbl:
    def __init__(self):
        self.port = serial.Serial()
        self.port.port = PORT
        self.port.baudrate = 115200
        self.port.timeout = 0.1
        # Leave DTR/RTS low: on Arduino GRBL boards DTR resets the controller,
        # which forgets the machine position
        self.port.dtr = False
        self.port.rts = False
        self.port.open()
        time.sleep(0.3)
        self.port.reset_input_buffer()

    def read_line(self):
        line = self.port.readline().decode(errors="replace").strip()
        if "Estop is activated" in line:
            sys.exit("e-stop is pressed: twist it to release, then run again")
        if line.startswith("ALARM"):
            sys.exit(f"GRBL {line} (5 = probe found nothing, 1/2 = limit hit): home again")
        return line

    def send(self, line, timeout=10.0):
        """Send one line and wait for its ok; returns the [...] lines before it"""
        self.port.write((line + "\n").encode())
        extras = []
        end = time.time() + timeout
        while time.time() < end:
            reply = self.read_line()
            if reply == "ok":
                return extras
            if reply.startswith("error"):
                sys.exit(f"GRBL {reply} on: {line}")
            if reply.startswith("["):
                extras.append(reply)
        sys.exit(f"no ok within {timeout:.0f} s for: {line}")

    def status(self):
        """State, machine position and active pins from a ? report"""
        self.port.write(b"?")
        end = time.time() + 2.0
        while time.time() < end:
            reply = self.read_line()
            if reply.startswith("<"):
                fields = reply.strip("<>").split("|")
                report = {"state": fields[0], "pins": ""}
                for field in fields[1:]:
                    name, _, value = field.partition(":")
                    if name == "MPos":
                        report["mpos"] = [float(part) for part in value.split(",")]
                    elif name == "Pn":
                        report["pins"] = value
                return report
        sys.exit("no status report from GRBL")

    def wait_idle(self, timeout=120.0):
        end = time.time() + timeout
        while time.time() < end:
            report = self.status()
            if report["state"] == "Idle":
                return report
            time.sleep(0.2)
        sys.exit(f"not idle after {timeout:.0f} s")

    def work_offset(self):
        """G54 in machine coordinates. Refuses a G92 offset: we never set one,
        so one there means someone else did and our sums would be off"""
        offsets = {}
        for line in self.send("$#"):
            name, _, value = line.strip("[]").partition(":")
            numbers = value.split(":")[0].split(",")
            if len(numbers) == 3:
                offsets[name] = [float(part) for part in numbers]
        if any(offsets["G92"]):
            sys.exit(f"G92 offset is set ({offsets['G92']}): clear it with G92.1")
        return offsets["G54"]


def work_position(grbl):
    report = grbl.wait_idle()
    offset = grbl.work_offset()
    return [machine - origin for machine, origin in zip(report["mpos"], offset)]


def probe_down(grbl, depth, feed=PROBE_FEED):
    """G38.2 down to `depth` (work Z, absolute); the work Z where it touched"""
    extras = grbl.send(f"G90 G38.2 Z{depth:.3f} F{feed}", timeout=120)
    for line in extras:
        if line.startswith("[PRB:"):
            machine = [float(part) for part in line[5:].split(":")[0].split(",")]
            return machine[2] - grbl.work_offset()[2]
    sys.exit(f"probe reported no position: {extras}")


def require_probe_open(grbl):
    if "P" in grbl.status()["pins"]:
        sys.exit("probe already closed: is the clip touching the copper or the bed?")


# --- commands, in the order a job uses them ---


def cmd_status(grbl, args):
    report = grbl.status()
    print(report)
    if report["state"] == "Idle":
        print("work position:", [round(value, 3) for value in work_position(grbl)])


def cmd_home(grbl, args):
    print("homing: Z up, then X and Y together (Y runs on alone for a while)")
    grbl.send("$H", timeout=120)
    print(grbl.wait_idle())


def cmd_jog(grbl, args):
    grbl.send(f"$J=G91 G21 X{args.dx} Y{args.dy} Z{args.dz} F{JOG_FEED}")
    grbl.wait_idle()
    print("work position:", [round(value, 3) for value in work_position(grbl)])


def wait_for_tap(grbl):
    """Prove the probe circuit works before driving the bit down: a fallen-off
    clip would never close it, and the search would plunge the bit into the
    board. The user taps the puck on the bit: closed, then open again"""
    require_probe_open(grbl)
    print(f"tap the puck against the bit (clip on the bit), within {TAP_WAIT} s ...", flush=True)
    end = time.time() + TAP_WAIT
    seen_closed = False
    while time.time() < end:
        closed = "P" in grbl.status()["pins"]
        if closed and not seen_closed:
            print("  probe closed", flush=True)
            seen_closed = True
        if seen_closed and not closed:
            print("  probe open again: the circuit works")
            return
        time.sleep(0.05)
    sys.exit("no tap seen: check the clip and the puck's plug, then run zero again")


def cmd_zero(grbl, args):
    wait_for_tap(grbl)
    print("now lay the puck metal face down on the copper, clear of the job area; starting in 10 s", flush=True)
    time.sleep(10)
    require_probe_open(grbl)
    grbl.send("G10 L20 P1 X0 Y0")
    # Search down from wherever we are: Z0 here isn't known yet, so relative
    grbl.send(f"G91 G38.2 Z-{ZERO_SEARCH} F{ZERO_FAST_FEED}", timeout=120)
    grbl.send(f"G0 Z{ZERO_BACK_OFF}")
    grbl.send(f"G38.2 Z-{ZERO_BACK_OFF * 2} F{ZERO_SLOW_FEED}", timeout=60)
    grbl.send("G10 L20 P1 Z0")
    grbl.send(f"G90 G0 Z{SAFE_Z}")
    print(f"work origin set: X0 Y0 here, Z0 on the copper; bit parked {SAFE_Z} mm above")


def cmd_probe(grbl, args):
    require_probe_open(grbl)
    columns = math.ceil(args.width / args.step) + 1
    rows = math.ceil(args.height / args.step) + 1
    grid_x = [args.width * column / (columns - 1) for column in range(columns)]
    grid_y = [args.height * row / (rows - 1) for row in range(rows)]
    heights = [[0.0] * columns for _ in range(rows)]

    grbl.send("G90 G21")
    grbl.send(f"G0 Z{SAFE_Z}")
    # Snake across the rows: no rapid back to the left edge each time
    for row, y_mm in enumerate(grid_y):
        order = range(columns) if row % 2 == 0 else reversed(range(columns))
        for column in order:
            grbl.send(f"G0 X{grid_x[column]:.3f} Y{y_mm:.3f}")
            heights[row][column] = probe_down(grbl, PROBE_DEPTH, args.feed)
            # Each point as it comes: a failing point shows a trend first
            print(f"  ({grid_x[column]:5.1f}, {y_mm:5.1f}) {heights[row][column]:+.3f}", flush=True)
            grbl.send(f"G0 Z{PROBE_CLEARANCE}")
    grbl.send(f"G0 Z{SAFE_Z}")

    with open(args.map, "w") as out:
        json.dump({"x": grid_x, "y": grid_y, "z": heights}, out, indent=1)
    # Top row last, so it prints like the board lies on the bed (Y away from you)
    for row in reversed(range(rows)):
        print(f"y {grid_y[row]:6.1f} " + " ".join(f"{z_mm:+.3f}" for z_mm in heights[row]))
    flat = [z_mm for row in heights for z_mm in row]
    print(f"spread {max(flat) - min(flat):.3f} mm -> {args.map}")


def height_at(height_map, x_mm, y_mm):
    """Bilinear between the four grid points around (x, y); clamped to the edge"""
    def locate(points, value):
        value = min(max(value, points[0]), points[-1])
        index = min(max(sum(1 for point in points if point <= value) - 1, 0), len(points) - 2)
        span = points[index + 1] - points[index]
        return index, (value - points[index]) / span if span else 0.0

    column, across = locate(height_map["x"], x_mm)
    row, along = locate(height_map["y"], y_mm)
    heights = height_map["z"]
    front = heights[row][column] * (1 - across) + heights[row][column + 1] * across
    back = heights[row + 1][column] * (1 - across) + heights[row + 1][column + 1] * across
    return front * (1 - along) + back * along


def bend(lines, height_map, start):
    """The job's lines with every G0/G1 Z raised by the map's height there.
    Cuts are split into SEGMENT pieces so Z follows the copper between points"""
    refused = {
        "G2": "arcs", "G3": "arcs", "G20": "inches", "G91": "relative moves",
        "G81": "drill cycles (pcb2gcode nog81=true)", "G83": "drill cycles",
        "M6": "tool changes (one tool per file)", "M0": "pauses (one tool per file)",
    }
    position = list(start)
    motion = None
    bent = []
    for number, raw in enumerate(lines, 1):
        code = re.sub(r"\(.*?\)|;.*", "", raw).strip().upper()
        if not code:
            continue
        words = re.findall(r"([A-Z])\s*([-+]?[\d.]+)", code)
        for letter, value in words:
            name = f"{letter}{float(value):g}"
            if name in refused:
                sys.exit(f"line {number}: {raw.strip()}: {refused[name]} not supported")
        for letter, value in words:
            if letter == "G" and float(value) in (0, 1):
                motion = int(float(value))
        axes = {letter: float(value) for letter, value in words if letter in "XYZ"}
        if not axes or motion is None:
            bent.append(code)
            continue
        target = [axes.get("X", position[0]), axes.get("Y", position[1]), axes.get("Z", position[2])]
        feed = next((f" F{value}" for letter, value in words if letter == "F"), "")
        run = math.dist(position[:2], target[:2])
        pieces = max(1, math.ceil(run / SEGMENT)) if motion == 1 else 1
        for piece in range(1, pieces + 1):
            point = [start_axis + (end_axis - start_axis) * piece / pieces
                     for start_axis, end_axis in zip(position, target)]
            z_mm = point[2] + height_at(height_map, point[0], point[1])
            bent.append(f"G{motion} X{point[0]:.3f} Y{point[1]:.3f} Z{z_mm:.4f}" + (feed if piece == 1 else ""))
        position = target
    return bent


def cmd_run(grbl, args):
    with open(args.map) as source:
        height_map = json.load(source)
    # A dry run never touches the machine: pretend we start at the origin
    start = [0.0, 0.0, 0.0] if args.dry else work_position(grbl)
    with open(args.job) as source:
        lines = bend(source.read().splitlines(), height_map, start)
    print(f"{len(lines)} lines after bending")
    if args.dry:
        print("\n".join(lines))
        return

    # Character counting: keep GRBL's receive buffer full without overflowing
    # it, so the planner never starves between short segments
    queued = collections.deque()
    done = 0
    next_report = 0.0

    def pump():
        nonlocal done, next_report
        if time.time() >= next_report:
            grbl.port.write(b"?")
            next_report = time.time() + 2.0
        reply = grbl.read_line()
        if reply == "ok":
            queued.popleft()
            done += 1
        elif reply.startswith("error"):
            grbl.port.write(b"!")
            sys.exit(f"GRBL {reply} at line {done + 1}: {lines[done]} (feed held)")
        elif reply.startswith("<"):
            print(f"{done}/{len(lines)} {reply}", flush=True)

    try:
        for line in lines:
            data = (line + "\n").encode()
            while sum(queued) + len(data) > RX_BUFFER:
                pump()
            grbl.port.write(data)
            queued.append(len(data))
        while queued:
            pump()
        grbl.wait_idle(timeout=600)
        print("job done")
    except KeyboardInterrupt:
        grbl.port.write(b"!")
        time.sleep(0.5)
        grbl.port.write(b"\x18")
        sys.exit("stopped: feed hold + reset. Spindle off, position lost: home again")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("status").set_defaults(run=cmd_status)
    commands.add_parser("home").set_defaults(run=cmd_home)
    jog = commands.add_parser("jog")
    jog.add_argument("dx", type=float)
    jog.add_argument("dy", type=float)
    jog.add_argument("dz", type=float, nargs="?", default=0.0)
    jog.set_defaults(run=cmd_jog)
    commands.add_parser("zero").set_defaults(run=cmd_zero)
    probe = commands.add_parser("probe")
    probe.add_argument("width", type=float)
    probe.add_argument("height", type=float)
    probe.add_argument("map")
    probe.add_argument("--step", type=float, default=10.0)
    # Slower for a V-bit's fragile tip: 10 mm/min touched without harm all day
    # 2026-10-16; 50 snapped one on unscrubbed copper
    probe.add_argument("--feed", type=float, default=PROBE_FEED, help="mm/min")
    probe.set_defaults(run=cmd_probe)
    run = commands.add_parser("run")
    run.add_argument("job")
    run.add_argument("map")
    run.add_argument("--dry", action="store_true", help="print the bent job, send nothing")
    run.set_defaults(run=cmd_run)
    args = parser.parse_args()
    dry = getattr(args, "dry", False)
    args.run(None if dry else Grbl(), args)


if __name__ == "__main__":
    main()
