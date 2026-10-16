"""K1 jack test coupon: a plate with the measured jack holes (pcb/k1/k1_jacks.json).

Both jacks should drop in together and still take the K1 plug. If they do,
the footprints are right before anything is milled; if not, the plate shows
which hole is off. Print it top face up: the top face is the component side.

    .venv/bin/python models/k1_coupon.py   ->   models/k1_coupon.stl
"""

import json
from pathlib import Path

from build123d import Box, BuildPart, Cylinder, Locations, Mode, export_stl

OUT = Path(__file__).parent
JACKS = OUT.parent / "pcb" / "k1" / "k1_jacks.json"

THICKNESS = 1.6  # the FR4 the breakout will be milled from
# One hole size for every pin, both jacks: the 3.5's 1.32 mm tabs need about
# 1.7 (FDM holes print small), and the 2.5's smaller pins get the same, not a
# smaller one: with 5 pins + 3 posts lining up at once, the slack has to cover
# the measurement error (~0.2 mm). Also one drill size = one drilling job
PIN_HOLE = 1.7
POST_HOLE = 1.8  # the 2.5's plastic posts are about 1 mm, located to about 0.5 mm
ENGRAVE = 0.3  # body outlines cut into the top face, this deep
GROOVE = 0.4  # and this wide
MARGIN = 2.0  # plate beyond the bodies, except at the front

jacks = json.loads(JACKS.read_text())
corners = jacks["3.5"]["body"] + jacks["2.5"]["body"]
left = min(x_mm for x_mm, _ in corners) - MARGIN
right = max(x_mm for x_mm, _ in corners) + MARGIN
# The front edge sits at the bodies' front faces, so the noses hang over it
# the way they will over the board edge
front = min(y_mm for _, y_mm in corners)
back = max(y_mm for _, y_mm in corners) + MARGIN


def at(x_mm, y_mm, z_mm):
    """The footprint's frame has y growing away from the plug (drawing
    convention, y down); in 3D seen from above, that's -y"""
    return (x_mm, -y_mm, z_mm)


with BuildPart() as coupon:
    with Locations(at((left + right) / 2, (front + back) / 2, THICKNESS / 2)):
        Box(right - left, back - front, THICKNESS)
    for jack in ("3.5", "2.5"):
        for pin in jacks[jack]["pins"]:
            with Locations(at(pin["x"], pin["y"], THICKNESS / 2)):
                Cylinder(radius=PIN_HOLE / 2, height=THICKNESS, mode=Mode.SUBTRACT)
    for x_mm, y_mm in jacks["2.5"]["posts"]:
        with Locations(at(x_mm, y_mm, THICKNESS / 2)):
            Cylinder(radius=POST_HOLE / 2, height=THICKNESS, mode=Mode.SUBTRACT)
    for jack in ("3.5", "2.5"):
        outline = jacks[jack]["body"]
        for (x0, y0), (x1, y1) in zip(outline, outline[1:] + outline[:1]):
            length = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
            across_x = abs(x1 - x0) > abs(y1 - y0)
            with Locations(at((x0 + x1) / 2, (y0 + y1) / 2, THICKNESS - ENGRAVE / 2)):
                Box(length if across_x else GROOVE, GROOVE if across_x else length, ENGRAVE, mode=Mode.SUBTRACT)

size = coupon.part.bounding_box().size
print(f"coupon {size.X:.1f} x {size.Y:.1f} x {size.Z:.1f} mm")
export_stl(coupon.part, str(OUT / "k1_coupon.stl"))
