"""MAX9814 amp board test coupon: a plate with the rev 10.1 holes for it.

The amp board (the purple Amazon clone) lies flat on the back of our board,
its 5 header pins through our holes, and 2 flying leads from its capsule
pads (Q1, capsule removed) down into our MIC_IN holes. The pins should drop
in, the board should sit inside the engraved outline, and the MIC_IN holes
should land right under Q1. Measured 2026-11-06 off a graph-paper photo the
user marked up (~/bench-sessions/2026-11-06-recorder/mic-board-2-crop-annotated.png).
Print it top face up: the top face is the side the amp board lies on.

    .venv/bin/python models/mic_coupon.py   ->   models/mic_coupon.stl
"""

from pathlib import Path

from build123d import Box, BuildPart, Cylinder, Locations, Mode, export_stl

OUT = Path(__file__).parent

THICKNESS = 1.6  # our board
PITCH = 2.54
PINS = 5  # AR, Out, Gain, Vdd, GND
# From the pin row (y = 0, AR at x = 0), toward the capsule end (+y)
Q1_FROM_PINS = 18.7  # the capsule pads, centred on the Gain pin, 2.54 apart
FAR_EDGE = 23.5  # the board's capsule end
NEAR_EDGE = 1.6  # behind the pins (not marked; read off the photo)
SIDE_EDGE = 1.8  # past AR and past GND
# FDM holes print small: 0.64 mm square header pins and the leads need ~1.3
PIN_HOLE = 1.3
ENGRAVE = 0.3  # the amp board's outline, cut into the top face this deep
GROOVE = 0.4  # and this wide
MARGIN = 3.0  # plate beyond the outline

gain_x = 2 * PITCH
left, right = -SIDE_EDGE, (PINS - 1) * PITCH + SIDE_EDGE
near, far = -NEAR_EDGE, FAR_EDGE

with BuildPart() as coupon:
    with Locations(((left + right) / 2, (near + far) / 2, THICKNESS / 2)):
        Box(right - left + 2 * MARGIN, far - near + 2 * MARGIN, THICKNESS)
    holes = [(pin * PITCH, 0) for pin in range(PINS)]
    holes += [(gain_x - PITCH / 2, Q1_FROM_PINS), (gain_x + PITCH / 2, Q1_FROM_PINS)]
    for x_mm, y_mm in holes:
        with Locations((x_mm, y_mm, THICKNESS / 2)):
            Cylinder(radius=PIN_HOLE / 2, height=THICKNESS, mode=Mode.SUBTRACT)
    outline = [(left, near), (right, near), (right, far), (left, far)]
    for (x0, y0), (x1, y1) in zip(outline, outline[1:] + outline[:1]):
        across_x = abs(x1 - x0) > abs(y1 - y0)
        length = abs(x1 - x0) if across_x else abs(y1 - y0)
        with Locations(((x0 + x1) / 2, (y0 + y1) / 2, THICKNESS - ENGRAVE / 2)):
            Box(length if across_x else GROOVE, GROOVE if across_x else length, ENGRAVE,
                mode=Mode.SUBTRACT)

size = coupon.part.bounding_box().size
print(f"coupon {size.X:.1f} x {size.Y:.1f} x {size.Z:.1f} mm")
export_stl(coupon.part, str(OUT / "mic_coupon.stl"))
