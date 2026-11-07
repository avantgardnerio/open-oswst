#!/usr/bin/python3
"""The board's holes and outlines, for the printed board coupon.

Reads the routed board (out/route/open-oswst.kicad_pcb, written by make.py)
and writes out/coupon.json: every hole (plated or not) with its drill, the
board's edge, and the silkscreen outlines on each side (lines and circles:
the parts' bodies). models/board_coupon.py turns that into a plate.

System Python on purpose, like make.py: KiCad's pcbnew module is installed
for it, not for the repo's venv.

Usage: pcb/coupon_data.py
"""

import json
from pathlib import Path

import pcbnew

PCB = Path(__file__).resolve().parent
ROUTED = PCB / "out" / "route" / "open-oswst.kicad_pcb"
OUT = PCB / "out" / "coupon.json"

# KiCad's F is the lid side, B the floor side (the Heltec's)
SILK_SIDES = {"F.Silkscreen": "lid", "B.Silkscreen": "floor"}  # KiCad 9's names


def mm(value):
    return round(pcbnew.ToMM(value), 3)


def main():
    board = pcbnew.LoadBoard(str(ROUTED))

    holes = []
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            drill = pad.GetDrillSize()
            if drill.x <= 0:
                continue
            position = pad.GetPosition()
            holes.append({"x": mm(position.x), "y": mm(position.y), "drill": mm(drill.x),
                          "part": footprint.GetReference()})

    edge = board.GetBoardEdgesBoundingBox()
    outline = {"x": mm(edge.GetX()), "y": mm(edge.GetY()),
               "width": mm(edge.GetWidth()), "height": mm(edge.GetHeight())}

    lines, circles = [], []
    for drawing in board.GetDrawings():
        side = SILK_SIDES.get(drawing.GetLayerName())
        if side is None or drawing.GetClass() != "PCB_SHAPE":
            continue
        shape = drawing.GetShape()
        if shape == pcbnew.SHAPE_T_CIRCLE:
            centre = drawing.GetCenter()
            circles.append({"side": side, "x": mm(centre.x), "y": mm(centre.y),
                            "radius": mm(drawing.GetRadius())})
        elif shape == pcbnew.SHAPE_T_RECT:
            start, end = drawing.GetStart(), drawing.GetEnd()
            x0, y0, x1, y1 = mm(start.x), mm(start.y), mm(end.x), mm(end.y)
            corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            for a, b in zip(corners, corners[1:] + corners[:1]):
                lines.append({"side": side, "from": a, "to": b})
        elif shape == pcbnew.SHAPE_T_SEGMENT:
            start, end = drawing.GetStart(), drawing.GetEnd()
            lines.append({"side": side, "from": (mm(start.x), mm(start.y)),
                          "to": (mm(end.x), mm(end.y))})

    OUT.write_text(json.dumps({"outline": outline, "holes": holes, "lines": lines,
                               "circles": circles}, indent=1))
    print(f"{len(holes)} holes, {len(lines)} outline lines, {len(circles)} circles -> {OUT}")


if __name__ == "__main__":
    main()
