#!/usr/bin/env python3
"""Grow each net's copper outward, keeping one tool width between nets.

    grow.py [out.png]

The copper we keep is what matters, not the cuts. Every net starts as its
pads and tracks; then all nets grow together in small steps. A net may not
come closer than one cut width (CUT) to any other net: at exactly CUT apart
the two nets' isolation cuts overlap 100% (one groove serves both), and any
closer the groove would eat copper. Growth stops at the chosen amount, or
earlier wherever a neighbour is in the way.

Geometry comes from pcb/k1/board.py and k1_jacks.json (top view, mm).
Polygon work is Clipper (Vatti clipping + offsetting) via pyclipper, on
integer micrometres.
"""

import json
import math
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import pyclipper  # noqa: E402

K1 = Path(__file__).parent.parent / "k1"
sys.path.insert(0, str(K1))
import board  # noqa: E402

SCALE = 1000  # Clipper works in integers: micrometres
CUT = 0.17  # mm: the V-bit's groove at 0.2 mm deep (tip 0.1 + 2 x 0.2 x tan 10)
STEP = 0.05  # mm of growth per round
EDGE_MARGIN = 0.5  # mm of bare board left inside the outline
GROWTH = [0.0, 0.5, 1.0, 3.0]  # mm: the panels drawn


def to_int(points):
    return [(round(x_mm * SCALE), round(y_mm * SCALE)) for x_mm, y_mm in points]


def offset(paths, distance_mm, open_paths=False):
    if not paths:
        return []
    clipper = pyclipper.PyclipperOffset()
    clipper.ArcTolerance = 0.005 * SCALE
    kind = pyclipper.ET_OPENROUND if open_paths else pyclipper.ET_CLOSEDPOLYGON
    clipper.AddPaths(paths, pyclipper.JT_ROUND, kind)
    return clipper.Execute(distance_mm * SCALE)


def boolean(subject, clip, operation):
    if not subject:
        return []
    clipper = pyclipper.Pyclipper()
    clipper.AddPaths(subject, pyclipper.PT_SUBJECT, True)
    if clip:
        clipper.AddPaths(clip, pyclipper.PT_CLIP, True)
    return clipper.Execute(operation, pyclipper.PFT_NONZERO, pyclipper.PFT_NONZERO)


def circle(x_mm, y_mm, diameter):
    sides = 48
    return [(x_mm + diameter / 2 * math.cos(2 * math.pi * side / sides),
             y_mm + diameter / 2 * math.sin(2 * math.pi * side / sides)) for side in range(sides)]


def nets_from_board():
    """Each net's copper: its jack pad, its header pad, its track"""
    jacks = json.loads((K1 / "k1_jacks.json").read_text())
    nets = {}
    for header, jack, number, waypoints in board.ROUTES:
        pin = jacks[jack]["pins"][number - 1]
        pad = board.PIN_35 if jack == "3.5" else board.PIN_25
        end = (board.HEADER_FIRST_X + (header - 1) * board.HEADER_PITCH, board.HEADER_Y)
        track = offset([to_int([(pin["x"], pin["y"]), *waypoints, end])], board.TRACK / 2, open_paths=True)
        pads = [to_int(circle(pin["x"], pin["y"], pad["size"])), to_int(circle(*end, board.HEADER["size"]))]
        nets[f"{jack}-{number}"] = boolean(pads, track, pyclipper.CT_UNION)
    return nets


def grow(nets, amount, inside):
    """All nets grow STEP at a time up to `amount`. Each step, a net may only
    take ground at least CUT + STEP from where the others were: they grow STEP
    too, so CUT stays between them afterwards"""
    grown = dict(nets)
    for _ in range(round(amount / STEP)):
        previous = dict(grown)
        for name in grown:
            others = [path for other, paths in previous.items() if other != name for path in paths]
            candidate = offset(previous[name], STEP)
            candidate = boolean(candidate, offset(others, CUT + STEP), pyclipper.CT_DIFFERENCE)
            candidate = boolean(candidate, inside, pyclipper.CT_INTERSECTION)
            # Never lose the original copper (a clip can't shrink a net)
            grown[name] = boolean(candidate, nets[name], pyclipper.CT_UNION)
    return grown


def narrowest_gap(grown):
    """Smallest distance between two nets, mm, found by growing them apart"""
    names = list(grown)
    for tenth in range(1, 40):
        distance = tenth * 0.01
        halves = {name: offset(paths, distance / 2) for name, paths in grown.items()}
        for index, name in enumerate(names):
            for other in names[index + 1:]:
                if boolean(halves[name], halves[other], pyclipper.CT_INTERSECTION):
                    return distance
    return None


def main():
    png = sys.argv[1] if len(sys.argv) > 1 else str(K1 / "out" / "growth.png")
    nets = nets_from_board()
    left, front, right, back = board.OUTLINE
    inside = [to_int([(left + EDGE_MARGIN, front + EDGE_MARGIN), (right - EDGE_MARGIN, front + EDGE_MARGIN),
                      (right - EDGE_MARGIN, back - EDGE_MARGIN), (left + EDGE_MARGIN, back - EDGE_MARGIN)])]
    colours = plt.cm.tab10.colors

    figure, panels = plt.subplots(1, len(GROWTH), figsize=(5 * len(GROWTH), 5.2))
    for panel, amount in zip(panels, GROWTH):
        grown = grow(nets, amount, inside)
        panel.add_patch(plt.Rectangle((left, front), right - left, back - front, color="#2d5a27"))  # fibreglass
        for index, (name, paths) in enumerate(grown.items()):
            for path in paths:
                points = [(x_um / SCALE, y_um / SCALE) for x_um, y_um in path]
                panel.add_patch(plt.Polygon(points, closed=True, color=colours[index % 10], alpha=0.9, lw=0))
        for name, paths in nets.items():
            for path in paths:
                points = [(x_um / SCALE, y_um / SCALE) for x_um, y_um in path]
                panel.add_patch(plt.Polygon(points, closed=True, fill=False, ec="black", lw=0.4))
        gap = narrowest_gap(grown)
        panel.set_title(f"grown {amount} mm: narrowest gap {gap:.2f} mm" if gap else f"grown {amount} mm")
        panel.set_xlim(left - 0.5, right + 0.5)
        panel.set_ylim(back + 0.5, front - 0.5)
        panel.set_aspect("equal")
        panel.set_xticks([])
        panel.set_yticks([])
        print(f"grown {amount} mm: narrowest gap {gap} mm", flush=True)
    figure.suptitle(f"Each net grown until it is one cut ({CUT} mm) from its neighbours; "
                    "black outline = the original pads and tracks; green = what gets milled away", fontsize=11)
    figure.savefig(png, dpi=110, bbox_inches="tight")
    print(png)


if __name__ == "__main__":
    main()
