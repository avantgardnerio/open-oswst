#!/usr/bin/env python3
"""Draw the measured K1 jack footprints (top view): writes k1_jacks_top.png.

    plot.py [k1_jacks.json] [k1_jacks_top.png]
"""

import json
import math
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import Circle, Polygon  # noqa: E402

COLOURS = {"3.5": "tab:blue", "2.5": "tab:orange"}
SLOT_WIDTH = 0.35  # mm: the blades' thickness, not measured; drawing only
POST_MM = 1.0  # the 2.5's plastic posts, from the close-up: about 1 mm


def main():
    json_path = sys.argv[1] if len(sys.argv) > 1 else "pcb/k1/k1_jacks.json"
    png_path = sys.argv[2] if len(sys.argv) > 2 else "pcb/k1/k1_jacks_top.png"
    with open(json_path) as handle:
        jacks = json.load(handle)

    figure, axes = plt.subplots(figsize=(8, 8.5))
    for jack, colour in COLOURS.items():
        axes.add_patch(Polygon(jacks[jack]["body"], closed=True, fill=False, ec=colour, lw=1.5, ls="--"))
        (back_x, back_y), (tip_x, tip_y) = jacks[jack]["bore"]
        axes.plot([back_x, tip_x], [back_y, tip_y], color=colour, lw=0.8, ls=":")
        axes.annotate(f"{jack} mm plug", (tip_x, tip_y), (tip_x, tip_y - 1.6), ha="center", color=colour,
                      arrowprops={"arrowstyle": "->", "color": colour})
        for number, pin in enumerate(jacks[jack]["pins"], 1):
            axes.add_patch(slot(pin, colour))
            # Number beside the slot, clear of a horizontal slot's end
            reach = pin["slot_mm"] / 2 if pin["slot_deg"] == 0 else SLOT_WIDTH / 2
            axes.text(pin["x"] + reach + 0.25, pin["y"] + 0.25, f"{number}", fontsize=8, color=colour)
    for x_mm, y_mm in jacks["2.5"]["posts"]:
        axes.add_patch(Circle((x_mm, y_mm), POST_MM / 2, fill=False, ec="gray"))

    axes.set_aspect("equal")
    axes.grid(True, lw=0.3)
    axes.set_xlim(-8, 18)
    axes.set_ylim(20, -3.5)  # y down, like KiCad: the board edge at the top
    axes.set_xlabel("mm")
    axes.set_ylabel("mm")
    slots = ", ".join(f"{jack}: {jacks[jack]['pins'][0]['slot_mm']} mm" for jack in COLOURS)
    axes.set_title(f"K1 jack pair, TOP view (component side)\nslots {slots}; bores {jacks['bore_gap_mm']} mm apart\n"
                   "dashed: body outline, gray: 2.5 posts (both from the close-up, about 0.5 mm)", fontsize=10)
    figure.savefig(png_path, dpi=130, bbox_inches="tight")
    print(png_path)


def slot(pin, colour):
    """The pin's slot as a filled rectangle: slot_mm long at slot_deg"""
    angle = math.radians(pin["slot_deg"])
    along = (math.cos(angle) * pin["slot_mm"] / 2, math.sin(angle) * pin["slot_mm"] / 2)
    across = (-math.sin(angle) * SLOT_WIDTH / 2, math.cos(angle) * SLOT_WIDTH / 2)
    corners = [(pin["x"] + along_sign * along[0] + across_sign * across[0],
                pin["y"] + along_sign * along[1] + across_sign * across[1])
               for along_sign, across_sign in ((1, 1), (1, -1), (-1, -1), (-1, 1))]
    return Polygon(corners, closed=True, color=colour)


if __name__ == "__main__":
    main()
