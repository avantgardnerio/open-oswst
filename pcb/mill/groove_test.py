#!/usr/bin/env python3
"""Groove test for the V-bit on copper clad: writes groove_test.nc.

    groove_test.py [groove_test.nc]

Four rows of grooves, one depth per row, and in each row three feeds. Under
a loupe, pick the shallowest depth that cuts cleanly through the copper
everywhere (no copper left in the groove bottom) at the fastest feed that
still leaves a clean edge. The V-bit's groove widens with depth (about
0.1 mm tip + 2 x depth x tan(10 deg)), so deeper isn't better.

Work origin = the front-left corner of the test area, Z0 = the copper (cnc.py
zero). The whole pattern fits in 22 x 13.5 mm: probe that area first
(cnc.py probe 22 14 map.json --step 5).
"""

import sys

DEPTHS = [0.05, 0.08, 0.11, 0.15]  # mm into the copper, one row each (front to back)
FEEDS = [100, 200, 300]  # mm/min along the groove, one groove each (left to right)
GROOVE = 5.0  # mm long
GROOVE_GAP = 2.0  # mm between grooves in a row
ROW_GAP = 3.5  # mm between rows
MARGIN = 1.5  # mm from the origin to the first groove
SAFE_Z = 2.0  # mm above the copper between grooves
PLUNGE_FEED = 30  # mm/min: slow, so the fragile tip enters without a bump
SPINDLE = 10000  # S10000 = full power ($30); the 775 motor tops out near 9000 rpm
SPIN_UP = 3  # s for the spindle to reach speed before the first plunge


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "pcb/mill/groove_test.nc"
    lines = [
        "(V-bit groove test: rows = depths " + " ".join(f"{depth}" for depth in DEPTHS)
        + " mm, columns = feeds " + " ".join(f"{feed}" for feed in FEEDS) + " mm/min)",
        "G21 G90 G94",
        f"G0 Z{SAFE_Z}",
        f"M3 S{SPINDLE}",
        f"G4 P{SPIN_UP}",
    ]
    for row, depth in enumerate(DEPTHS):
        y_mm = MARGIN + row * ROW_GAP
        for column, feed in enumerate(FEEDS):
            x_start = MARGIN + column * (GROOVE + GROOVE_GAP)
            lines += [
                f"(depth {depth} mm, feed {feed} mm/min)",
                f"G0 X{x_start:.3f} Y{y_mm:.3f}",
                f"G1 Z{-depth:.3f} F{PLUNGE_FEED}",
                f"G1 X{x_start + GROOVE:.3f} F{feed}",
                f"G0 Z{SAFE_Z}",
            ]
    lines += ["M5", "G0 Z5", "G0 X0 Y0"]
    with open(path, "w") as handle:
        handle.write("\n".join(lines) + "\n")
    width = MARGIN * 2 + len(FEEDS) * GROOVE + (len(FEEDS) - 1) * GROOVE_GAP
    depth_mm = MARGIN * 2 + (len(DEPTHS) - 1) * ROW_GAP
    print(f"{path}: {len(lines)} lines, pattern {width:.1f} x {depth_mm:.1f} mm")


if __name__ == "__main__":
    main()
