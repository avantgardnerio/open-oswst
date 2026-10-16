#!/usr/bin/env python3
"""Draw what G-code jobs will cut, from the G-code itself: the check before
anything runs on the mill.

    plot_gcode.py out.png job.ngc [job.ngc ...]

Cutting moves (Z below 0) are solid lines, one colour per job; plunges that
don't move sideways (drilling) are circles; rapids above the work are faint.
Coordinates are the job's own work coordinates, seen from above the mill.
"""

import re
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

COLOURS = ["tab:red", "tab:blue", "tab:green", "tab:purple", "tab:orange"]


def moves(path):
    """(start, end, rapid) for every G0/G1 move, in work coordinates"""
    position = [0.0, 0.0, 10.0]
    motion = 0
    for raw in open(path):
        code = re.sub(r"\(.*?\)|;.*", "", raw).strip().upper()
        words = dict((letter, float(value)) for letter, value in re.findall(r"([GXYZ])\s*([-+]?[\d.]+)", code)
                     if letter != "G")
        for value in re.findall(r"G\s*0*([0-9]+)\b", code):
            if value in ("0", "1"):
                motion = int(value)
        if not words:
            continue
        target = [words.get("X", position[0]), words.get("Y", position[1]), words.get("Z", position[2])]
        yield position, target, motion == 0
        position = target


def main():
    png, jobs = sys.argv[1], sys.argv[2:]
    figure, axes = plt.subplots(figsize=(9, 8))
    for index, job in enumerate(jobs):
        colour = COLOURS[index % len(COLOURS)]
        label = job.split("/")[-1]
        cuts = holes = 0
        # A job that cuts sideways plunges only to start each cut: mark plunges
        # as holes only in jobs that never move sideways while cutting (drilling)
        drilling = not any(end[2] < 0 and (start[0], start[1]) != (end[0], end[1])
                           for start, end, _ in moves(job))
        for start, end, rapid in moves(job):
            sideways = (start[0], start[1]) != (end[0], end[1])
            if end[2] < 0 and sideways:
                axes.plot([start[0], end[0]], [start[1], end[1]], color=colour, lw=1,
                          label=label if cuts == 0 else None)
                cuts += 1
            elif drilling and end[2] < 0 and start[2] >= 0:
                axes.plot(end[0], end[1], "o", mfc="none", color=colour, ms=7,
                          label=label if holes == 0 and cuts == 0 else None)
                holes += 1
            elif rapid and sideways and index == 0:
                axes.plot([start[0], end[0]], [start[1], end[1]], color="0.85", lw=0.5, zorder=0)
        print(f"{label}: {cuts} cutting moves, {holes} plunges")
    axes.set_aspect("equal")
    axes.grid(True, lw=0.3)
    axes.legend(loc="upper right", fontsize=8)
    axes.set_xlabel("work X, mm (+ = to your right)")
    axes.set_ylabel("work Y, mm (+ = toward the back of the board)")
    axes.set_title("Toolpaths as the mill will cut them (seen from above, copper side up)", fontsize=10)
    figure.savefig(png, dpi=120, bbox_inches="tight")
    print(png)


if __name__ == "__main__":
    main()
