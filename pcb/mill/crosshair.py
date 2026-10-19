#!/usr/bin/env python3
"""A V-bit crosshair: the reference the router bit is centred on.

    crosshair.py OUT.ngc X Y [ARM_MM]

Engraved by the V-bit right after the traces (same bit, same alignment), just
outside the board. Its arms (default 3 mm each way, 6 mm across) stick out
past the 3.175 mm router bit, so centring the bit on the cross by eye is
unambiguous and good to ~0.1 mm: a flat-ended router bit can't centre itself
in a drilled hole the way a drill point or V-bit can.
"""

import sys

DEPTH = -0.2  # mm, like the traces: through the copper, visible
FEED = 150  # mm/min
PLUNGE = 30  # mm/min
SAFE_Z = 2.0


def main():
    path, x_mm, y_mm = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
    arm = float(sys.argv[4]) if len(sys.argv) > 4 else 3.0
    lines = ["(crosshair for centring the router bit)", "G21 G90 G94", f"G0 Z{SAFE_Z}", "M3 S10000", "G4 P3"]
    for (x0, y0), (x1, y1) in (((x_mm - arm, y_mm), (x_mm + arm, y_mm)), ((x_mm, y_mm - arm), (x_mm, y_mm + arm))):
        lines += [f"G0 X{x0:.3f} Y{y0:.3f}", f"G1 Z{DEPTH} F{PLUNGE}", f"G1 X{x1:.3f} Y{y1:.3f} F{FEED}", f"G0 Z{SAFE_Z}"]
    lines += ["M5", "G0 Z5"]
    with open(path, "w") as handle:
        handle.write("\n".join(lines) + "\n")
    print(f"{path}: cross at ({x_mm}, {y_mm}), {2 * arm} mm across")


if __name__ == "__main__":
    main()
