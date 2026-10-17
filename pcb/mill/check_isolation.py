#!/usr/bin/env python3
"""Check an isolation job really isolates: simulate the cut on a raster.

    check_isolation.py ISOLATION.ngc GROOVE_MM HEADER_Y DRILL.ngc [DRILL.ngc ...]

Starts from a full sheet of copper, removes a GROOVE_MM wide band along every
cutting move (Z below 0), then labels the copper that's left into connected
pieces. Each plated hole (the drill jobs' plunges) must sit in copper, and
every piece holding a header pad (holes near HEADER_Y) must hold exactly one
other hole on its own: a net that shares its piece with another net is a
short; a pad left with no copper is cut through. Holes the jobs drill but no
pad needs (posts) may share a piece or sit on bare board.
"""

import re
import sys

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

PIXEL = 0.01  # mm per raster cell


def moves(path):
    position = [0.0, 0.0, 10.0]
    motion = 0
    for raw in open(path):
        code = re.sub(r"\(.*?\)|;.*", "", raw).strip().upper()
        for value in re.findall(r"G\s*0*([0-9]+)\b", code):
            if value in ("0", "1"):
                motion = int(value)
        words = {letter: float(value) for letter, value in re.findall(r"([XYZ])\s*([-+]?[\d.]+)", code)}
        if not words:
            continue
        target = [words.get("X", position[0]), words.get("Y", position[1]), words.get("Z", position[2])]
        yield position, target, motion == 0
        position = target


def main():
    isolation, groove, header_y = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
    holes = []
    for drill in sys.argv[4:]:
        holes += [(end[0], end[1]) for start, end, _ in moves(drill) if end[2] < 0 <= start[2]]

    cuts = [(start, end) for start, end, _ in moves(isolation) if end[2] < 0 and start[2] < 0 or
            (end[2] < 0 and (start[0], start[1]) != (end[0], end[1]))]
    xs = [point[0] for cut in cuts for point in cut] + [hole[0] for hole in holes]
    ys = [point[1] for cut in cuts for point in cut] + [hole[1] for hole in holes]
    left, bottom = min(xs) - 2, min(ys) - 2
    width, height = int((max(xs) + 2 - left) / PIXEL), int((max(ys) + 2 - bottom) / PIXEL)

    def pixel(x_mm, y_mm):
        return ((x_mm - left) / PIXEL, (y_mm - bottom) / PIXEL)

    # Copper everywhere, then the grooves: round-ended bands GROOVE_MM wide
    sheet = Image.new("1", (width, height), 1)
    draw = ImageDraw.Draw(sheet)
    radius = groove / 2 / PIXEL
    for start, end in cuts:
        draw.line([pixel(*start[:2]), pixel(*end[:2])], fill=0, width=max(1, round(2 * radius)))
        for point in (start, end):
            x_px, y_px = pixel(*point[:2])
            draw.ellipse([x_px - radius, y_px - radius, x_px + radius, y_px + radius], fill=0)
    copper = np.array(sheet, dtype=bool)
    pieces, count = ndimage.label(copper)

    found = {}
    bare = []
    for x_mm, y_mm in holes:
        x_px, y_px = pixel(x_mm, y_mm)
        piece = pieces[int(round(y_px)), int(round(x_px))]
        if piece == 0:
            bare.append((x_mm, y_mm))
        else:
            found.setdefault(piece, []).append((x_mm, y_mm))

    headers = [hole for hole in holes if abs(hole[1] - header_y) < 0.3]
    problems = 0
    print(f"{count} copper pieces; {len(holes)} holes, {len(headers)} on the header row")
    for header in sorted(headers):
        piece = next((key for key, members in found.items() if header in members), None)
        if piece is None:
            print(f"  PROBLEM header pad at {header}: no copper left around it")
            problems += 1
            continue
        partners = [hole for hole in found[piece] if hole != header]
        other_headers = [hole for hole in partners if hole in headers]
        if other_headers:
            print(f"  PROBLEM header pad at {header} is SHORTED to header pad(s) {other_headers}")
            problems += 1
        print(f"  header ({header[0]:6.2f}, {header[1]:5.2f}) -> " +
              ", ".join(f"({x_mm:6.2f}, {y_mm:5.2f})" for x_mm, y_mm in partners))
    if bare:
        print(f"  holes on bare board (fine for posts): {bare}")
    print("ISOLATION OK" if problems == 0 else f"{problems} PROBLEM(S)")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
