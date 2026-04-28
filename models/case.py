"""OSWST case — hello world box, two-piece (bottom + lid)."""

from build123d import *
from pathlib import Path

OUT = Path(__file__).parent

# Overall dimensions
WIDTH = 80
LENGTH = 125
HEIGHT = 30
WALL = 2
FILLET_R = 3
LID_HEIGHT = 8  # how much of the total height is lid

# Amp screw post
AMP_POST_FROM_TOP = 8.5    # mm from top inside wall (Y axis)
AMP_POST_FROM_LEFT = 11    # mm from left inside wall (X axis)
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

# Screw post locations
floor_z = -HEIGHT / 2 + WALL
OVERLAP = 0.5  # sink into floor for clean boolean fusion

# Amp posts — positioned from inside walls
post_x = -(WIDTH / 2 - WALL - AMP_POST_FROM_LEFT)
post_y1 = LENGTH / 2 - WALL - AMP_POST_FROM_TOP
post_y2 = post_y1 - 33  # 2nd post 33mm below

# Perfboard posts — 50x70mm board, landscape (70mm across width, 50mm along length)
PERF_W = 70
PERF_L = 50
PERF_HOLE_FROM_LR = 4.5   # mm from left & right edges of board
PERF_HOLE_FROM_TB = 2      # mm from top & bottom edges of board
PERF_POST_HEIGHT = AMP_POST_HEIGHT
PERF_GAP_FROM_AMP = 8      # mm below lower amp post

perf_center_y = post_y2 - PERF_GAP_FROM_AMP - PERF_L / 2
perf_center_x = 0  # centered in case

# Collect all post locations: (x, y, height)
all_posts = []
for py in [post_y1, post_y2]:
    all_posts.append((post_x, py, AMP_POST_HEIGHT))
for px in [perf_center_x - PERF_W / 2 + PERF_HOLE_FROM_LR,
           perf_center_x + PERF_W / 2 - PERF_HOLE_FROM_LR]:
    for py in [perf_center_y + PERF_L / 2 - PERF_HOLE_FROM_TB,
               perf_center_y - PERF_L / 2 + PERF_HOLE_FROM_TB]:
        all_posts.append((px, py, PERF_POST_HEIGHT))

# Add all posts then drill all holes
posts_solid = None
for px, py, h in all_posts:
    cz = floor_z + (h - OVERLAP) / 2
    post = Pos(px, py, cz) * Cylinder(radius=AMP_POST_OD / 2, height=h + OVERLAP)
    posts_solid = post if posts_solid is None else (posts_solid + post)

holes_solid = None
for px, py, h in all_posts:
    cz = floor_z + h / 2
    hole = Pos(px, py, cz) * Cylinder(radius=AMP_POST_ID / 2, height=h + 1)
    holes_solid = hole if holes_solid is None else (holes_solid + hole)

result = bottom + posts_solid - holes_solid
bottom = result.solids()[0] if hasattr(result, 'solids') else result

# SMA hole through top wall (positive Y), aligned with posts
SMA_HOLE_DIA = 6.5
sma_z = floor_z + AMP_POST_HEIGHT  # top of posts
sma_hole = Pos(post_x, LENGTH / 2, sma_z) * Rot(90, 0, 0) * Cylinder(
    radius=SMA_HOLE_DIA / 2, height=WALL * 3  # oversized to cut clean through
)
bottom = bottom - sma_hole

# Top wall controls — laid out right of antenna (positive X direction)
# Antenna right edge is at post_x + 6.5 (13mm dia)
ant_right_x = post_x + 6.5
controls_z = sma_z  # same Z height as SMA hole

# Power slider cutout: 11x6mm hole, 20mm total with screwdowns
SLIDER_W = 11   # along X (slide direction)
SLIDER_H = 6    # along Z
slider_center_x = ant_right_x + 20 / 2  # 20mm footprint starts at antenna edge

# Encoder holes: 7mm dia, 15mm knobs, 3mm gap between knobs
ENCODER_DIA = 7
KNOB_DIA = 15
KNOB_GAP = 3
enc1_center_x = ant_right_x + 20 + KNOB_GAP + KNOB_DIA / 2
enc2_center_x = enc1_center_x + KNOB_DIA / 2 + KNOB_GAP + KNOB_DIA / 2

# Cut slider
slider_hole = Pos(slider_center_x, LENGTH / 2, controls_z) * Rot(90, 0, 0) * Box(
    SLIDER_W, WALL * 3, SLIDER_H
)
bottom = bottom - slider_hole

# Cut encoder holes
for enc_x in [enc1_center_x, enc2_center_x]:
    enc_hole = Pos(enc_x, LENGTH / 2, controls_z) * Rot(90, 0, 0) * Cylinder(
        radius=ENCODER_DIA / 2, height=WALL * 3
    )
    bottom = bottom - enc_hole

# PTT button hole through right wall (positive X), 13mm from top wall
PTT_DIA = 16
ptt_y = LENGTH / 2 - 31  # 31mm from top (antenna end) — split the diff, tune after print
ptt_z = -2.5  # centered on usable wall height, clears floor and rim
ptt_hole = Pos(WIDTH / 2, ptt_y, ptt_z) * Rot(0, 90, 0) * Cylinder(
    radius=PTT_DIA / 2, height=WALL * 3
)
bottom = bottom - ptt_hole

# Kenwood jack through left wall (negative X), 3.5mm on top, 2.5mm below, 12mm apart
KENWOOD_SPACING = 12
kenwood_y_top = -(LENGTH / 2 - 45)  # 3.5mm jack 45mm from bottom
kenwood_z = ptt_z
for jack_dia, y_off in [(3.5, 0), (2.5, -KENWOOD_SPACING)]:
    jack = Pos(-WIDTH / 2, kenwood_y_top + y_off, kenwood_z) * Rot(0, 90, 0) * Cylinder(
        radius=jack_dia / 2, height=WALL * 3
    )
    bottom = bottom - jack

# USB-C hole through right wall (positive X)
USBC_W = 9    # along Y
USBC_H = 4    # along Z
usbc_top_z = floor_z + AMP_POST_HEIGHT + 10     # upper Z edge
usbc_center_z = usbc_top_z - USBC_H / 2
perf_top_post_y = perf_center_y + PERF_L / 2 - PERF_HOLE_FROM_TB
usbc_base_y = perf_top_post_y - 22                # lower Y edge, 22mm below top perfboard post
usbc_center_y = usbc_base_y + USBC_W / 2
usbc_hole = Pos(WIDTH / 2, usbc_center_y, usbc_center_z) * Box(
    WALL * 3, USBC_W, USBC_H  # oversized in X to cut clean through
)
bottom = bottom - usbc_hole

# Screen hole through floor (negative Z face), 33x19mm
SCREEN_W = 33   # along X
SCREEN_H = 19   # along Y
perf_left_post_x = perf_center_x - PERF_W / 2 + PERF_HOLE_FROM_LR
screen_center_x = perf_left_post_x + 45
screen_center_y = usbc_center_y  # centered on USB-C hole
screen_hole = Pos(screen_center_x, screen_center_y, -HEIGHT / 2) * Box(
    SCREEN_W, SCREEN_H, WALL * 3  # oversized in Z to cut clean through
)
bottom = bottom - screen_hole

# Speaker hole through floor (negative Z face), 28x28mm
SPEAKER_SIZE = 28
PA_BOARD_W = 26
inner_left = -WIDTH / 2 + WALL
speaker_center_x = inner_left + PA_BOARD_W + (WIDTH - 2 * WALL - PA_BOARD_W) / 2
speaker_center_y = LENGTH / 2 - 12 - SPEAKER_SIZE / 2  # 12mm from top
speaker_hole = Pos(speaker_center_x, speaker_center_y, -HEIGHT / 2) * Box(
    SPEAKER_SIZE, SPEAKER_SIZE, WALL * 3
)
bottom = bottom - speaker_hole

# Speaker screw holes, 36mm apart, centered on speaker
for sx in [-18, 18]:
    screw = Pos(speaker_center_x + sx, speaker_center_y, -HEIGHT / 2) * Cylinder(
        radius=AMP_POST_ID / 2, height=WALL * 3
    )
    bottom = bottom - screw

# Mic hole through floor, 53mm below top perfboard post, 4mm right of left post
MIC_DIA = 10
mic_x = 0  # centered horizontally
mic_y = perf_top_post_y - 53
mic_hole = Pos(mic_x, mic_y, -HEIGHT / 2) * Cylinder(
    radius=MIC_DIA / 2, height=WALL * 3
)
bottom = bottom - mic_hole

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
