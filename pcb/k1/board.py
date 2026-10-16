#!/usr/bin/env python3
"""K1 jack breakout: both Kenwood K1 jacks on a small single-sided board,
every jack pin routed to a 10-pin 0.1" header that plugs into a breadboard.

    .venv/bin/python pcb/k1/board.py   ->   pcb/k1/k1-breakout.kicad_pcb

Milled in-house, so: copper on the bottom only, 0.5 mm tracks, 0.4 mm
clearance (k1-breakout.kicad_dru), two drill sizes. The parts sit on top;
the header's long pins point down into the breadboard. All 10 jack pins come
out, unnamed: which pin is tip/ring/sleeve/switch gets mapped on the bench.
Jack positions come from the measured k1_jacks.json (top view, mm, origin =
the 3.5 bore at its nose tip, plugs coming in from -y), shifted by OFFSET.
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE.parent))
from kicad import Board  # noqa: E402

OFFSET = (30.0, 30.0)  # the footprint frame sits here on the KiCad page
TRACK = 0.5
# Pads: ring = (pad - drill) / 2. The 2.5's pins are only 1.9-2.0 mm apart,
# so its pads stay at 1.5 to leave 0.4 mm between them
PIN_35 = {"size": 2.6, "drill": 1.7}  # 1.32 mm tabs; 1.7 fit the test plate
PIN_25 = {"size": 1.5, "drill": 1.1}  # 0.76 mm pins
POST_DRILL = 1.7  # the 2.5's plastic posts: holes, no copper
HEADER = {"size": 1.8, "drill": 1.1}  # 0.64 mm square pins
HEADER_Y = 21.0
HEADER_PITCH = 2.54
HEADER_FIRST_X = -6.43  # 10 pins centred under the two jacks
OUTLINE = (-8.2, 2.5, 18.4, 23.5)  # left, front, right, back; front = the jack bodies' faces

# Which header pin each jack pin goes to, and the route between them (jack
# pad -> waypoints -> header pad). Pins are numbered as in k1_jacks.json.
# Lanes go back in left-to-right order, so nothing crosses on one layer
ROUTES = [
    # header, jack, pin, waypoints
    (1, "3.5", 2, [(-7.0, 11.0), (-7.0, 20.0)]),  # round the outside of 3.5 pin 1
    (2, "3.5", 1, []),
    (3, "3.5", 3, []),
    (4, "3.5", 4, []),
    (5, "3.5", 5, []),
    (6, "2.5", 2, [(6.2, 6.6), (6.2, 9.2), (7.3, 11.6), (7.3, 19.6)]),  # left of 2.5 pin 1, right of 3.5 pin 5
    (7, "2.5", 1, [(8.6, 9.0), (8.6, 19.6)]),  # behind the post at (9.32, 7.02)
    (8, "2.5", 3, [(11.2, 5.6), (11.2, 19.6)]),  # between the two post columns
    (9, "2.5", 4, [(15.02, 19.4)]),
    (10, "2.5", 5, [(16.5, 5.9), (16.5, 19.6)]),  # round the outside of 2.5 pin 4
]


def main():
    jacks = json.loads((HERE / "k1_jacks.json").read_text())
    board = Board()

    def at(x_mm, y_mm):
        return (round(x_mm + OFFSET[0], 3), round(y_mm + OFFSET[1], 3))

    def net(jack, number):
        return f"J{jack.replace('.', '')}_{number}"

    for jack, pad_style in (("3.5", PIN_35), ("2.5", PIN_25)):
        pads = []
        for number, pin in enumerate(jacks[jack]["pins"], 1):
            x_mm, y_mm = at(pin["x"], pin["y"])
            pads.append({"name": str(number), "x": x_mm, "y": y_mm, **pad_style,
                         "net": net(jack, number), "layers": "B.Cu B.Mask"})
        if jack == "2.5":
            for x_mm, y_mm in jacks["2.5"]["posts"]:
                post_x, post_y = at(x_mm, y_mm)
                pads.append({"name": "", "x": post_x, "y": post_y, "drill": POST_DRILL, "plated": False})
        board.add_footprint(f"K1_Jack_{jack}mm_measured", 0, 0, pads, label=f"J{jack}")

    header_pads = []
    for header, jack, number, _ in ROUTES:
        x_mm, y_mm = at(HEADER_FIRST_X + (header - 1) * HEADER_PITCH, HEADER_Y)
        header_pads.append({"name": str(header), "x": x_mm, "y": y_mm, **HEADER,
                            "net": net(jack, number), "layers": "B.Cu B.Mask"})
    board.add_footprint("PinHeader_1x10_P2.54mm_milled", 0, 0, header_pads, label="H")

    for header, jack, number, waypoints in ROUTES:
        pin = jacks[jack]["pins"][number - 1]
        start = (pin["x"], pin["y"])
        end = (HEADER_FIRST_X + (header - 1) * HEADER_PITCH, HEADER_Y)
        board.add_track([at(*point) for point in [start, *waypoints, end]], TRACK, net(jack, number))

    left, front, right, back = OUTLINE
    corner_x, corner_y = at(left, front)
    board.add_rect(corner_x, corner_y, right - left, back - front)
    board.save(HERE / "k1-breakout.kicad_pcb")


if __name__ == "__main__":
    main()
