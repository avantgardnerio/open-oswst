"""OSWST case — hello world box, two-piece (bottom + lid)."""

from build123d import *
from pathlib import Path

OUT = Path(__file__).parent

# Overall dimensions
WIDTH = 80
# 145 -> 140: the south end had 7.80mm of dead space past the MIC daughterboard, which is the
# southernmost part in the case (it hangs 12.1mm off the PCB's south edge). Trimmed 5mm, leaving
# 2.80mm of margin for the mic wiring. Worth doing because the case only just fits its pouch.
#
# Everything here chains off LENGTH/2 (post_y1 -> post_y2 -> perf_center_y -> perf_top_post_y ->
# mic_y/screen/usbc), so shrinking LENGTH walks the whole board assembly south by half the trim
# while the south wall walks north by the other half: the south gap closes 1:1 with the trim, and
# every north-end clearance (the RF/antenna section) is unchanged. Verified with fitcheck.py.
LENGTH = 140
HEIGHT = 34
WALL = 2
FILLET_R = 3
LID_HEIGHT = 4  # how much of the total height is lid

# Amp screw post
AMP_POST_FROM_TOP = 8.5    # mm from top inside wall (Y axis)
AMP_POST_FROM_LEFT = 18    # mm from left inside wall (X axis) — shifted inward to clear 4th lid post
AMP_POST_HEIGHT = 18      # tops sit 6mm below case side tops
AMP_POST_OD = 4.5          # outer diameter — trimmed for module clearance
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
post_y2 = post_y1 - 34.6  # 2nd post 34.6mm below (moved up 0.9mm to clear board)

# Perfboard posts — 50x70mm board, landscape (70mm across width, 50mm along length)
PERF_W = 70
PERF_L = 50
PERF_HOLE_FROM_LR = 5      # mm from left & right edges of board
PERF_HOLE_FROM_TB = 2.5    # mm from top & bottom edges of board
PERF_POST_HEIGHT = 11
PERF_GAP_FROM_AMP = 28     # mm below lower amp post

perf_center_y = post_y2 - PERF_GAP_FROM_AMP - PERF_L / 2
perf_center_x = 0  # centered in case
perf_top_post_y = perf_center_y + PERF_L / 2 - PERF_HOLE_FROM_TB

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

# SMA hole through top wall (positive Y), aligned with raised amp posts
SMA_HOLE_DIA = 7.0
sma_z = floor_z + AMP_POST_HEIGHT  # top of raised amp posts
sma_hole = Pos(post_x - 5, LENGTH / 2, sma_z) * Rot(90, 0, 0) * Cylinder(
    radius=SMA_HOLE_DIA / 2, height=WALL * 3
)
bottom = bottom - sma_hole

# Top wall controls — laid out right of antenna (positive X direction)
# Antenna right edge — fixed position, independent of amp post shift
ant_right_x = -23.5
controls_z = floor_z + 11  # original control height — independent of raised amp/SMA

# Power slider cutout: 11x6mm hole, 20mm total with screwdowns
SLIDER_W = 7    # along X (slide direction) — just the slider nub
SLIDER_H = 3.5  # along Z
slider_center_x = ant_right_x + 20 / 2 + 5  # shifted 5mm toward encoders

# Encoder holes: 7mm dia, 15mm knobs, 3mm gap between knobs
ENCODER_DIA = 7.2
KNOB_DIA = 15
KNOB_GAP = 3
enc1_center_x = ant_right_x + 20 + KNOB_GAP + KNOB_DIA / 2
enc2_center_x = enc1_center_x + KNOB_DIA / 2 + KNOB_GAP + KNOB_DIA / 2

# Cut slider
slider_z = controls_z - 3  # shifted 3mm toward bed
slider_hole = Pos(slider_center_x, LENGTH / 2, slider_z) * Rot(90, 0, 0) * Box(
    SLIDER_W, WALL * 3, WALL * 5
)
bottom = bottom - slider_hole

# Lanyard loop — half-circle protruding from top wall
LANYARD_DIA = 14       # diameter of semicircle
LANYARD_THICK = 6      # Z thickness
lanyard_x = slider_center_x  # aligned with power switch center
lanyard_z = sma_z             # same Z as SMA hole center
lanyard_cyl = Pos(lanyard_x, LENGTH / 2, lanyard_z) * Cylinder(
    radius=LANYARD_DIA / 2, height=LANYARD_THICK
)
# Remove inner half — keep only the part protruding beyond the wall
lanyard_cut = Pos(lanyard_x, LENGTH / 2 - LANYARD_DIA / 2, lanyard_z) * Box(
    LANYARD_DIA + 2, LANYARD_DIA, LANYARD_THICK + 2
)
lanyard_loop = lanyard_cyl - lanyard_cut
# Taper bottom face: 6mm thick at wall → 2mm at outer edge (no supports needed)
LANYARD_THIN = 2
taper_plane = Plane(
    origin=(lanyard_x, LENGTH / 2, lanyard_z - LANYARD_THICK / 2),
    z_dir=(0, -(LANYARD_THICK - LANYARD_THIN), LANYARD_DIA / 2),
)
lanyard_loop = split(lanyard_loop, taper_plane, keep=Keep.TOP)
# Drill hole through loop for lanyard — bottom of hole at wall surface
LANYARD_HOLE_DIA = 6
lanyard_hole = Pos(lanyard_x, LENGTH / 2 + LANYARD_HOLE_DIA / 2, lanyard_z) * Cylinder(
    radius=LANYARD_HOLE_DIA / 2, height=LANYARD_THICK + 2
)
bottom = bottom + (lanyard_loop - lanyard_hole)

# Power switch screw holes, 14mm apart
SLIDER_SCREW_SPACING = 14
for sx in [-1, +1]:
    screw = Pos(slider_center_x + sx * SLIDER_SCREW_SPACING / 2, LENGTH / 2, slider_z) * Rot(90, 0, 0) * Cylinder(
        radius=AMP_POST_ID / 2, height=WALL * 3
    )
    bottom = bottom - screw

# Cut encoder holes
for enc_x in [enc1_center_x, enc2_center_x]:
    enc_hole = Pos(enc_x, LENGTH / 2, controls_z) * Rot(90, 0, 0) * Cylinder(
        radius=ENCODER_DIA / 2, height=WALL * 3
    )
    bottom = bottom - enc_hole

# PTT button hole through right wall (positive X), 13mm from top wall
PTT_DIA = 16.5
ptt_y = LENGTH / 2 - 23  # 23mm from top (antenna end) — shifted 8mm toward encoders
ptt_z = (floor_z + split_z) / 2  # centered on usable wall height
ptt_hole = Pos(WIDTH / 2, ptt_y, ptt_z) * Rot(0, 90, 0) * Cylinder(
    radius=PTT_DIA / 2, height=WALL * 3
)
bottom = bottom - ptt_hole

# Kenwood jack through left wall (negative X), 3.5mm on top, 2.5mm below, 12mm apart
KENWOOD_SPACING = 12
kenwood_recess_w = 10                     # Z span of recess
kenwood_recess_h = KENWOOD_SPACING + 10  # Y span — covers both jacks with margin
kenwood_y_top = (perf_top_post_y + AMP_POST_OD / 2 + 3  # 3mm above top perf post edge
                 + KENWOOD_SPACING / 2 + kenwood_recess_h / 2)  # offset so recess bottom clears
kenwood_z = -HEIGHT / 2 + FILLET_R + kenwood_recess_w / 2 + 2  # recess clears fillet + 2mm margin
for jack_dia, y_off in [(6.4, 0), (4.6, -KENWOOD_SPACING)]:
    jack = Pos(-WIDTH / 2, kenwood_y_top + y_off, kenwood_z) * Rot(0, 90, 0) * Cylinder(
        radius=jack_dia / 2, height=WALL * 3
    )
    bottom = bottom - jack

# Kenwood recess — thin wall from 2mm to 1mm for nut clearance
KENWOOD_RECESS_DEPTH = 1.0   # mm to remove from outer wall
kenwood_mid_y = kenwood_y_top - KENWOOD_SPACING / 2  # midpoint between the two jacks
kenwood_recess = Pos(-WIDTH / 2, kenwood_mid_y, kenwood_z) * Box(
    KENWOOD_RECESS_DEPTH * 2, kenwood_recess_h, kenwood_recess_w
)
bottom = bottom - kenwood_recess

# USB-C hole through right wall (positive X) — centred on the Heltec's receptacle.
#
# Z: the Heltec's PCB front face sits at floor_z + PERF_POST_HEIGHT - 2.50 (its header plastic,
# trapped between the boards) - 1.60 (its own PCB) = -8.10, and the receptacle hangs below that
# on the component side, so its centre is a further 1.60 down. ⚠️ The 3.20mm receptacle height
# is a standard USB-C figure, not measured off this board.
#
# Y: the module's short-axis centreline, which the J2/J3 header rows fix exactly — the same line
# the screen sits on, so this legitimately equals screen_center_y now. It used to be aliased the
# other way round (screen_center_y = usbc_center_y) by coincidence rather than for a reason.
#
# Sized for the cable, not the receptacle: the overmold measures 11.2 x 6.0, and bigger cables
# want headroom. Kept clear of the bottom fillet (hole bottom lands at -13.95, fillet starts
# around -14.0).
USBC_W = 15.0   # along Y — 11.2 overmold + 1.9 per side
USBC_H = 8.5    # along Z — 6.0 overmold + 1.25 per side
usbc_center_z = floor_z + PERF_POST_HEIGHT - 2.50 - 1.60 - 1.60
usbc_center_y = perf_top_post_y - 18.5
usbc_hole = Pos(WIDTH / 2, usbc_center_y, usbc_center_z) * Box(
    WALL * 3, USBC_W, USBC_H  # oversized in X to cut clean through
)
bottom = bottom - usbc_hole

# Screen hole through floor (negative Z face) — centred on the Heltec's OLED glass.
#
# The glass is 33.00 x 18.60. That's the outer of two nested rectangles on the datasheet drawing
# (section 5); the inner 22.00 x 11.40 is only the lit area. Confirmed by measuring the physical
# screen at 33 x 18.7.
#
# It sits flush with the last FULL-WIDTH edge at the Heltec's antenna end — 3.81mm in from the
# tip, where the 45-degree chamfers forming the antenna extension begin. It can't be flush with
# the tip itself: the chamfers narrow the module to 25.40 - 2(3.81) = 17.78mm there, less than
# the 18.60 glass. Across the short axis it's centred, which the J2/J3 header rows fix exactly.
#
# Centre derived and checked by models/fitcheck.py. The old values (-0.50, usbc_center_y) were
# 5.82mm off in X and 1.50mm in Y, which left 3.82mm of the screen's USB-C edge behind solid
# floor — the "screen hole isn't quite right" seen on the physical build.
SCREEN_W = 34.5   # 33.00 glass + 0.75mm per side
SCREEN_H = 20.0   # 18.60 glass + 0.70mm per side
perf_left_post_x = perf_center_x - PERF_W / 2 + PERF_HOLE_FROM_LR
screen_center_x = 5.32                      # glass centre; fixed in X, nothing upstream moves it
screen_center_y = perf_top_post_y - 18.5    # follows the board, like mic_y
screen_hole = Pos(screen_center_x, screen_center_y, -HEIGHT / 2) * Box(
    SCREEN_W, SCREEN_H, WALL * 3  # oversized in Z to cut clean through
)
bottom = bottom - screen_hole

# Speaker hole through floor (negative Z face), 28x28mm
SPEAKER_W = 30    # X axis (side with ears)
SPEAKER_L = 33    # Y axis
PA_BOARD_W = 26
inner_left = -WIDTH / 2 + WALL
speaker_center_x = inner_left + PA_BOARD_W + (WIDTH - 2 * WALL - PA_BOARD_W) / 2
speaker_center_y = LENGTH / 2 - 32 - SPEAKER_L / 2  # 32mm from top (shifted 20mm down)
speaker_hole = Pos(speaker_center_x, speaker_center_y, -HEIGHT / 2) * Box(
    SPEAKER_W, SPEAKER_L, WALL * 3
)
bottom = bottom - speaker_hole

# Speaker screw posts, 36mm apart, centered on speaker
SPEAKER_POST_HEIGHT = PERF_POST_HEIGHT - WALL - 4  # shortened for speaker fit
for sx in [-18.5, 18.5]:
    px = speaker_center_x + sx
    py = speaker_center_y
    h = SPEAKER_POST_HEIGHT
    cz = floor_z + (h - OVERLAP) / 2
    post = Pos(px, py, cz) * Cylinder(radius=AMP_POST_OD / 2, height=h + OVERLAP)
    bottom = bottom + post
    cz2 = floor_z + h / 2
    hole = Pos(px, py, cz2) * Cylinder(radius=AMP_POST_ID / 2, height=h + 1)
    bottom = bottom - hole

# Mic hole through floor — centred on the MAX9814's capsule.
#
# board.py puts the MIC header at BY + BH - 9, which lands 38.5mm below the top perfboard post.
# The daughterboard hangs south off it (25.4 x 14.10, header row 4.30mm in from its near edge),
# putting the capsule a further 8.40mm south. Derived and checked by models/fitcheck.py.
#
# All measured off the physical parts: the capsule's near edge is 11.51mm from the board's
# pin-end edge and it is 9.69mm across, so its centre is 16.36mm from that edge; the pin row is
# 5.74mm in from the same edge. The capsule is NOT centred on the board and NOT aligned with the
# pin row.
#
# ⚠️ One caveat before printing: an earlier measurement put the pin row 21.10mm from the FAR
# edge, and 5.74 + 21.10 = 26.84 against a 25.40mm board — 1.44mm over. Resolving that shifts
# this hole by up to 1.44mm, and at 0.4mm clearance per side the hole no longer absorbs it.
MIC_CAPSULE_BELOW_HEADER = 16.36 - 5.74
MIC_DIA = 10.5   # 9.69 capsule + 0.4mm per side
mic_x = 0  # centred horizontally; the capsule is on the board's centreline
mic_capsule_y = perf_top_post_y - 38.5 - MIC_CAPSULE_BELOW_HEADER
# Pushed south until the hole's edge meets the floor-to-south-wall fillet, per the physical build.
# This puts the hole 7.03mm south of the modelled capsule centre.
mic_y = -(LENGTH / 2 - FILLET_R) + MIC_DIA / 2
mic_hole = Pos(mic_x, mic_y, -HEIGHT / 2) * Cylinder(
    radius=MIC_DIA / 2, height=WALL * 3
)
bottom = bottom - mic_hole

# Lid screw posts — 3 posts from floor to split plane, triangle layout
LID_POST_HEIGHT = split_z - floor_z  # full height to split plane
LID_CLEARANCE_DIA = 2.4              # clearance hole in lid for M2 screw

lid_post_inset = WALL + AMP_POST_OD / 2  # post center inset from outer wall
lid_post_locs = [
    (+(WIDTH / 2 - lid_post_inset), +(LENGTH / 2 - lid_post_inset)),   # top-right (by speaker)
    (+(WIDTH / 2 - lid_post_inset), -(LENGTH / 2 - lid_post_inset)),   # bottom-right (by mic)
    (-(WIDTH / 2 - lid_post_inset), -(LENGTH / 2 - lid_post_inset)),   # bottom-left (other side of mic)
    (-(WIDTH / 2 - lid_post_inset), +(LENGTH / 2 - lid_post_inset)),   # top-left (by amp)
]

for px, py in lid_post_locs:
    cz = floor_z + (LID_POST_HEIGHT - OVERLAP) / 2
    post = Pos(px, py, cz) * Cylinder(radius=AMP_POST_OD / 2, height=LID_POST_HEIGHT + OVERLAP)
    hole = Pos(px, py, cz) * Cylinder(radius=AMP_POST_ID / 2, height=LID_POST_HEIGHT + 1)
    bottom = bottom + post - hole

# Clearance holes + countersinks through lid for flat-head M2 screws
CSINK_DIA = 4.0  # M2 flat-head diameter
csink_depth = (CSINK_DIA - LID_CLEARANCE_DIA) / 2  # 90° cone depth
lid_hole_z = split_z + LID_HEIGHT / 2
lid_top_z = HEIGHT / 2
for px, py in lid_post_locs:
    hole = Pos(px, py, lid_hole_z) * Cylinder(radius=LID_CLEARANCE_DIA / 2, height=LID_HEIGHT + 1)
    csink = Pos(px, py, lid_top_z - csink_depth / 2) * Cone(
        bottom_radius=LID_CLEARANCE_DIA / 2, top_radius=CSINK_DIA / 2, height=csink_depth
    )
    lid = lid - hole - csink

# Lid corner guides — tabs extending from lid into case for alignment
GUIDE_LEN = 15       # mm along edge
GUIDE_DEPTH = 2.5    # mm extending below split into case body
GUIDE_THICK = 1.5    # mm wall thickness
GUIDE_GAP = 0.3      # mm print clearance from case inner walls

inner_hx = WIDTH / 2 - WALL - GUIDE_GAP   # clearance from case inner walls
inner_hy = LENGTH / 2 - WALL - GUIDE_GAP
lid_inner_z = HEIGHT / 2 - WALL           # inside ceiling of lid
LID_OVERLAP = 1.0                         # mm past lid ceiling for clean boolean union
guide_total = (lid_inner_z - split_z) + GUIDE_DEPTH + LID_OVERLAP
guide_z = split_z + guide_total / 2 - GUIDE_DEPTH  # top extends into lid floor, bottom protrudes into case

guides = None
for sx, sy in [(+1, +1), (+1, -1), (-1, +1), (-1, -1)]:
    # L-bracket at each corner: one tab along X-wall, one along Y-wall
    # Tab along X-wall (constrains Y)
    gx = sx * (inner_hx - GUIDE_THICK / 2)
    corner_inset = FILLET_R + 10  # clear screw posts in corners
    gy = sy * (inner_hy - GUIDE_LEN / 2 - corner_inset)
    tab_x = Pos(gx, gy, guide_z) * Box(GUIDE_THICK, GUIDE_LEN, guide_total)
    # Tab along Y-wall (constrains X)
    gx2 = sx * (inner_hx - GUIDE_LEN / 2 - corner_inset)
    gy2 = sy * (inner_hy - GUIDE_THICK / 2)
    tab_y = Pos(gx2, gy2, guide_z) * Box(GUIDE_LEN, GUIDE_THICK, guide_total)
    # Chamfer bottom edges of each tab individually before combining
    GUIDE_CHAMFER = 0.5
    guide_bottom_z = guide_z - guide_total / 2
    for tab in [tab_x, tab_y]:
        bottom_edges = [e for e in tab.edges() if abs(e.center().Z - guide_bottom_z) < 0.1]
        if bottom_edges:
            tab = chamfer(bottom_edges, length=GUIDE_CHAMFER)
        guides = tab if guides is None else (guides + tab)

# Battery cradle on lid interior — centered over perfboard
BATT_W = 55   # X (across width, matching perfboard landscape)
BATT_L = 35   # Y (along length)
BATT_H = 12   # Z thickness
BATT_WALL = 1.5  # retaining wall thickness
BATT_WALL_H = 8   # retaining wall height (enough to hold battery, not full height)

batt_x = perf_center_x
batt_y = perf_center_y - 15  # shifted 15mm down
BATT_WALL_EMBED = 1  # mm walls extend into lid for strong bond
batt_cradle_z = lid_inner_z - BATT_WALL_H / 2 + BATT_WALL_EMBED / 2  # embedded into lid

# Four retaining walls around the battery pocket
for dx, dy, ww, wl in [
    (-(BATT_W / 2 + BATT_WALL / 2), 0, BATT_WALL, BATT_L - 14),  # left
    (+(BATT_W / 2 + BATT_WALL / 2), 0, BATT_WALL, BATT_L - 14),  # right
    (0, -(BATT_L / 2 + BATT_WALL / 2), BATT_W - 14, BATT_WALL),  # bottom
    (0, +(BATT_L / 2 + BATT_WALL / 2), BATT_W - 14, BATT_WALL),  # top
]:
    wall = Pos(batt_x + dx, batt_y + dy, batt_cradle_z) * Box(ww, wl, BATT_WALL_H + BATT_WALL_EMBED)
    lid = lid + wall

# Inward-facing clips on X-axis (left/right) walls to retain battery
CLIP_DEPTH = 2.5   # how far clip protrudes inward (X)
CLIP_H = 1.5       # clip height (Z), protrudes below wall bottom
clip_len = BATT_L - 14  # same Y length as the X-axis walls
wall_bottom_z = lid_inner_z - BATT_WALL_H
clip_z = wall_bottom_z - CLIP_H / 2  # hangs below wall

for sign in [-1, +1]:
    # overlap wall horizontally so clip is connected
    clip_x = batt_x + sign * (BATT_W / 2 - CLIP_DEPTH / 2 + BATT_WALL / 2)
    clip = Pos(clip_x, batt_y, clip_z) * Box(CLIP_DEPTH + BATT_WALL, clip_len, CLIP_H)
    lid = lid + clip

lid = lid + guides

# Design-coordinate solids, before the print-plate shuffle below: origin at the centre of the
# outer box, floor at floor_z, +Y the antenna end. fitcheck.py needs these to place the PCB
# against the case, so keep them under separate names.
bottom_design = bottom
lid_design = lid

# Move both onto the bed (Z=0) and place lid next to bottom
bottom = Pos(0, 0, -bottom.bounding_box().min.Z) * bottom
lid = Rot(180, 0, 0) * lid  # flip so flat top is on bed
lid = Pos(WIDTH + 5, 0, -lid.bounding_box().min.Z) * lid

# Combine into single print plate
plate = Compound(children=[bottom, lid])

# Export — skipped on import (fitcheck.py) so it doesn't rewrite the tracked artifacts
if __name__ == "__main__":
    export_step(plate, str(OUT / "case.step"))
    export_stl(plate, str(OUT / "case.stl"))
    print(f"Exported case: {WIDTH}x{LENGTH}x{HEIGHT}mm, lid={LID_HEIGHT}mm, wall={WALL}mm")

# CQ-editor preview
if "show_object" in dir():
    import cadquery as cq
    show_object(cq.Shape.cast(bottom.wrapped), name="bottom")
    show_object(cq.Shape.cast(lid.wrapped), name="lid")
