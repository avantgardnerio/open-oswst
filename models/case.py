"""OSWST case — hello world box, two-piece (bottom + lid)."""

from build123d import *
from pathlib import Path

OUT = Path(__file__).parent

# Overall dimensions
WIDTH = 72
LENGTH = 110
HEIGHT = 30
WALL = 2
FILLET_R = 3
LID_HEIGHT = 8  # how much of the total height is lid

# Amp screw post
AMP_POST_FROM_TOP = 8.5    # mm from top inside wall (Y axis)
AMP_POST_FROM_RIGHT = 11   # mm from right inside wall (X axis)
AMP_POST_HEIGHT = 6
AMP_POST_OD = 5            # outer diameter
AMP_POST_ID = 1.8          # pilot hole for M2 screw

# Full outer shell
outer = Box(WIDTH, LENGTH, HEIGHT)
outer = fillet(outer.edges(), radius=FILLET_R)

# Split into bottom and lid using bisect
split_z = HEIGHT / 2 - LID_HEIGHT  # Z=0 is center, so split plane in world coords

bottom = split(outer, Plane(origin=(0, 0, split_z), z_dir=(0, 0, 1)), keep=Keep.BOTTOM)
lid = split(outer, Plane(origin=(0, 0, split_z), z_dir=(0, 0, 1)), keep=Keep.TOP)

# Hollow both — open at the split face
bottom_top_face = bottom.faces().sort_by(Axis.Z)[-1]
bottom = offset(bottom, amount=-WALL, openings=[bottom_top_face])

lid_bottom_face = lid.faces().sort_by(Axis.Z)[0]
lid = offset(lid, amount=-WALL, openings=[lid_bottom_face])

# Amp screw posts — positioned from inside walls
floor_z = -HEIGHT / 2 + WALL
post_x = WIDTH / 2 - WALL - AMP_POST_FROM_RIGHT
post_y1 = LENGTH / 2 - WALL - AMP_POST_FROM_TOP
post_y2 = post_y1 - 33  # 2nd post 33mm below

for py in [post_y1, post_y2]:
    post = Pos(post_x, py, floor_z + AMP_POST_HEIGHT / 2) * Cylinder(
        radius=AMP_POST_OD / 2, height=AMP_POST_HEIGHT
    )
    hole = Pos(post_x, py, floor_z + AMP_POST_HEIGHT / 2) * Cylinder(
        radius=AMP_POST_ID / 2, height=AMP_POST_HEIGHT
    )
    bottom = bottom + post - hole

# SMA hole through top wall (positive Y), aligned with posts
SMA_HOLE_DIA = 6.5
sma_z = floor_z + AMP_POST_HEIGHT  # top of posts
sma_hole = Pos(post_x, LENGTH / 2, sma_z) * Rot(90, 0, 0) * Cylinder(
    radius=SMA_HOLE_DIA / 2, height=WALL * 3  # oversized to cut clean through
)
bottom = bottom - sma_hole

# Move both onto the bed (Z=0) and place lid next to bottom
bottom = Pos(0, 0, -bottom.bounding_box().min.Z) * bottom
lid = Rot(180, 0, 0) * lid  # flip so flat top is on bed
lid = Pos(WIDTH + 5, 0, -lid.bounding_box().min.Z) * lid

# Combine into single print plate
plate = Compound(children=[bottom, lid])

# Export
export_step(plate, str(OUT / "case.step"))
export_stl(plate, str(OUT / "case.stl"))
print(f"Exported case: {WIDTH}x{LENGTH}x{HEIGHT}mm, lid={LID_HEIGHT}mm, wall={WALL}mm")

# CQ-editor preview
if "show_object" in dir():
    import cadquery as cq
    show_object(cq.Shape.cast(bottom.wrapped), name="bottom")
    show_object(cq.Shape.cast(lid.wrapped), name="lid")
