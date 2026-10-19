#!/usr/bin/env python3
"""Show where to centre the bit after a bit change: the board as it lies on the
mill (copper side up), with a big red marker on the reference point.

    show_reference.py OUT.png X Y "LABEL" JOB.ngc [JOB.ngc ...]

X Y: the reference in work coordinates (the home hole, or the crosshair).
The jobs are drawn for orientation only. Opens the image when done.

Why: after a bit change the new bit is centred by hand on one reference
point, and the job is re-anchored to it. A reference named in words ("the
pad at the end of the trace from pin 7") got misread once and put a whole
cut-out 7 mm off (2026-10-17). A picture can't be misread that way.
"""

import subprocess
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import Circle  # noqa: E402

from plot_gcode import moves  # noqa: E402


def main():
    png, x_mm, y_mm, label = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
    figure, axes = plt.subplots(figsize=(9, 8))
    for job in sys.argv[5:]:
        job_moves = list(moves(job))
        # Only drill jobs make holes; elsewhere a plunge just starts a groove
        drilling = not any(end[2] < 0 and (start[0], start[1]) != (end[0], end[1]) for start, end, _ in job_moves)
        for start, end, rapid in job_moves:
            sideways = (start[0], start[1]) != (end[0], end[1])
            if end[2] < 0 and sideways:
                axes.plot([start[0], end[0]], [start[1], end[1]], color="0.55", lw=0.8)
            elif drilling and end[2] < 0 <= start[2]:
                axes.add_patch(Circle(end[:2], 0.45, fill=False, ec="0.35", lw=0.8))

    # The reference: big, red, unmistakable
    axes.add_patch(Circle((x_mm, y_mm), 2.2, fill=False, ec="red", lw=4))
    axes.plot(x_mm, y_mm, marker="+", color="red", markersize=40, mew=3)
    axes.annotate(label, (x_mm, y_mm), (x_mm + 6, y_mm - 6), color="red", fontsize=14, weight="bold",
                  arrowprops={"arrowstyle": "->", "color": "red", "lw": 3})

    axes.set_aspect("equal")
    axes.autoscale()
    left, right = axes.get_xlim()
    bottom, top = axes.get_ylim()
    axes.set_xlim(min(left, x_mm - 4), max(right, x_mm + 14))
    axes.set_ylim(min(bottom, y_mm - 10), max(top, y_mm + 4))
    axes.grid(True, lw=0.3)
    axes.set_xlabel("work X, mm  (+ = to your RIGHT)")
    axes.set_ylabel("work Y, mm  (+ = toward the BACK of the machine)")
    axes.set_title("The board as it lies on the mill, copper side up (mirrored vs the layout drawing)\n"
                   "Centre the new bit on the RED mark", fontsize=11)
    figure.savefig(png, dpi=120, bbox_inches="tight")
    print(png)
    subprocess.Popen(["xdg-open", png], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()
