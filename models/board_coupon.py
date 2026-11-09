"""Whole-board test coupon: a plate the size of the PCB, with every hole and
every part outline, to fit the real parts on before a fab order.

The Heltec on its sockets, both amp boards, the K1 jacks, the capsule, C1,
the JSTs: if they all go in where the board says, and the plate drops into
the printed case, the layout is right. Made from the routed board itself
(pcb/coupon_data.py), so it always matches what would be ordered.

The top face is the lid side (KiCad's F), the bottom face the floor side
(B, the Heltec's): each side's outlines are engraved into its own face.
Print it lid side up.

    python3 pcb/coupon_data.py                       (after pcb/make.py)
    .venv/bin/python models/board_coupon.py   ->   models/board_coupon.stl
"""

import json
import math
from pathlib import Path

from build123d import Box, BuildPart, Cylinder, Locations, Mode, Rot, export_stl

OUT = Path(__file__).parent
DATA = OUT.parent / "pcb" / "out" / "coupon.json"

THICKNESS = 1.6  # the PCB
# FDM holes print small: each hole this much over its drill
HOLE_SLACK = 0.3
ENGRAVE = 0.3  # outlines cut into a face this deep
GROOVE = 0.4  # and this wide

data = json.loads(DATA.read_text())
edge = data["outline"]


def at(x_mm, y_mm, z_mm):
    """KiCad's y grows down the page; in 3D seen from the lid side, that's -y"""
    return (x_mm - edge["x"], -(y_mm - edge["y"]), z_mm)


def face_z(side):
    """The middle of the engraving's depth, on the given side's face"""
    return THICKNESS - ENGRAVE / 2 if side == "lid" else ENGRAVE / 2


with BuildPart() as coupon:
    with Locations(at(edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2,
                      THICKNESS / 2)):
        Box(edge["width"], edge["height"], THICKNESS)
    for line in data["lines"]:
        (x0, y0), (x1, y1) = line["from"], line["to"]
        length = math.hypot(x1 - x0, y1 - y0)
        if length < 0.01:
            continue
        # -y in 3D (at()), so the angle flips sign
        angle = -math.degrees(math.atan2(y1 - y0, x1 - x0))
        with Locations(at((x0 + x1) / 2, (y0 + y1) / 2, face_z(line["side"]))):
            Box(length + GROOVE, GROOVE, ENGRAVE, rotation=Rot(0, 0, angle), mode=Mode.SUBTRACT)
    for circle in data["circles"]:
        with Locations(at(circle["x"], circle["y"], face_z(circle["side"]))):
            Cylinder(radius=circle["radius"] + GROOVE / 2, height=ENGRAVE, mode=Mode.SUBTRACT)
            Cylinder(radius=circle["radius"] - GROOVE / 2, height=ENGRAVE, mode=Mode.ADD)
    # The holes last, and taller than the plate: cut before the circles, the
    # disc each ring's engraving adds back capped the holes inside it (the
    # capsule's, C1's), and cylinders exactly the plate's height leave skins
    # at the faces. Rev 10.1's coupon needed those holes drilled (user,
    # 2026-11-08)
    for hole in data["holes"]:
        with Locations(at(hole["x"], hole["y"], THICKNESS / 2)):
            Cylinder(radius=(hole["drill"] + HOLE_SLACK) / 2, height=THICKNESS + 2,
                     mode=Mode.SUBTRACT)

size = coupon.part.bounding_box().size
print(f"board coupon {size.X:.1f} x {size.Y:.1f} x {size.Z:.1f} mm, "
      f"{len(data['holes'])} holes")
export_stl(coupon.part, str(OUT / "board_coupon.stl"))
