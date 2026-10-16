#!/usr/bin/env python3
"""Draw the K1 breakout (top view, through the board): k1-breakout.png.

    .venv/bin/python pcb/k1/plot_board.py

Same numbers as board.py: jack outlines, pads, posts, tracks, header pins
with the jack pin each one carries.
"""

import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import Circle, Polygon, Rectangle  # noqa: E402

import board  # noqa: E402

HERE = Path(__file__).parent
COLOURS = {"3.5": "tab:blue", "2.5": "tab:orange"}
COPPER = "#c87533"


def main():
    jacks = json.loads((HERE / "k1_jacks.json").read_text())
    figure, axes = plt.subplots(figsize=(9, 8))
    left, front, right, back = board.OUTLINE
    axes.add_patch(Rectangle((left, front), right - left, back - front, fill=False, ec="black", lw=1.5))

    for header, jack, number, waypoints in board.ROUTES:
        pin = jacks[jack]["pins"][number - 1]
        end = (board.HEADER_FIRST_X + (header - 1) * board.HEADER_PITCH, board.HEADER_Y)
        points = [(pin["x"], pin["y"]), *waypoints, end]
        axes.plot(*zip(*points), color=COPPER, lw=board.TRACK * 6.5, solid_capstyle="round", zorder=2)
        axes.add_patch(Circle(end, board.HEADER["size"] / 2, color=COPPER, zorder=3))
        axes.add_patch(Circle(end, board.HEADER["drill"] / 2, color="white", zorder=4))
        axes.text(end[0], end[1] + 1.7, f"{header}\n{jack}-{number}", ha="center", va="top", fontsize=8,
                  color=COLOURS[jack])

    for jack, style in (("3.5", board.PIN_35), ("2.5", board.PIN_25)):
        axes.add_patch(Polygon(jacks[jack]["body"], closed=True, fill=False, ec=COLOURS[jack], ls="--", lw=1))
        for number, pin in enumerate(jacks[jack]["pins"], 1):
            axes.add_patch(Circle((pin["x"], pin["y"]), style["size"] / 2, color=COPPER, zorder=3))
            axes.add_patch(Circle((pin["x"], pin["y"]), style["drill"] / 2, color="white", zorder=4))
            axes.text(pin["x"], pin["y"], str(number), ha="center", va="center", fontsize=6, zorder=5)
        (_, _), (tip_x, tip_y) = jacks[jack]["bore"]
        axes.annotate(f"{jack} mm plug", (tip_x, tip_y + 2.2), (tip_x, tip_y - 1.5), ha="center",
                      color=COLOURS[jack], arrowprops={"arrowstyle": "->", "color": COLOURS[jack]})
    for x_mm, y_mm in jacks["2.5"]["posts"]:
        axes.add_patch(Circle((x_mm, y_mm), board.POST_DRILL / 2, fill=False, ec="black", lw=1, zorder=4))

    axes.set_aspect("equal")
    axes.set_xlim(left - 1.5, right + 1.5)
    axes.set_ylim(back + 2.5, front - 5)  # y down, like KiCad: plugs at the top
    axes.grid(True, lw=0.3)
    axes.set_xlabel("mm")
    axes.set_title(f"K1 breakout, {right - left:.1f} x {back - front:.1f} mm, TOP view through the board\n"
                   "copper on the bottom; dashed = jack bodies; black rings = unplated post holes;\n"
                   "header labels: header pin / jack-pin", fontsize=10)
    figure.savefig(HERE / "k1-breakout.png", dpi=120, bbox_inches="tight")
    print(HERE / "k1-breakout.png")


if __name__ == "__main__":
    main()
