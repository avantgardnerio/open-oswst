#!/usr/bin/env python3
"""A 1:1 paper template of the K1 jack holes: writes k1_jacks_1to1.pdf.

    template.py [k1_jacks.json] [k1_jacks_1to1.pdf]

Print at "Actual size" / 100 % (never "Fit to page"), then measure the scale
bar: it must be 50.0 mm. Hole circles are the coupon's sizes (slot + 0.4 mm),
seen from the top (component side), the plug coming in from the top edge.
"""

import json
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import Circle, Polygon  # noqa: E402

PAGE_MM = (215.9, 279.4)  # US Letter
CLEARANCE = 0.4  # as models/k1_coupon.py
POST_HOLE = 1.5
# Where the footprint's origin (the 3.5 bore at its nose tip) lands on the page
ORIGIN_MM = (90, 60)


def main():
    json_path = sys.argv[1] if len(sys.argv) > 1 else "pcb/k1/k1_jacks.json"
    pdf_path = sys.argv[2] if len(sys.argv) > 2 else "pcb/k1/k1_jacks_1to1.pdf"
    with open(json_path) as handle:
        jacks = json.load(handle)

    # One data unit = 1 mm: the axes fill the page exactly, with no margins
    figure = plt.figure(figsize=(PAGE_MM[0] / 25.4, PAGE_MM[1] / 25.4))
    axes = figure.add_axes((0, 0, 1, 1))
    axes.set_xlim(0, PAGE_MM[0])
    axes.set_ylim(PAGE_MM[1], 0)  # y down, like the footprint frame
    axes.axis("off")

    def page(x_mm, y_mm):
        return (ORIGIN_MM[0] + x_mm, ORIGIN_MM[1] + y_mm)

    for jack in ("3.5", "2.5"):
        axes.add_patch(Polygon([page(*corner) for corner in jacks[jack]["body"]], closed=True,
                               fill=False, ec="0.5", lw=0.6, ls="--"))
        for number, pin in enumerate(jacks[jack]["pins"], 1):
            centre = page(pin["x"], pin["y"])
            axes.add_patch(Circle(centre, (pin["slot_mm"] + CLEARANCE) / 2, fill=False, ec="black", lw=0.4))
            axes.plot(*centre, marker="+", color="black", markersize=3, mew=0.4)
        (_, _), (tip_x, tip_y) = jacks[jack]["bore"]
        axes.annotate(f"{jack} mm plug", page(tip_x, tip_y), page(tip_x, tip_y - 8), ha="center", fontsize=7,
                      arrowprops={"arrowstyle": "->", "lw": 0.6})
    for x_mm, y_mm in jacks["2.5"]["posts"]:
        axes.add_patch(Circle(page(x_mm, y_mm), POST_HOLE / 2, fill=False, ec="0.4", lw=0.4))

    # Scale check: 50 mm bar with 10 mm ticks
    bar_x, bar_y = 30, 110
    axes.plot([bar_x, bar_x + 50], [bar_y, bar_y], color="black", lw=0.8)
    for tick in range(0, 51, 10):
        axes.plot([bar_x + tick, bar_x + tick], [bar_y - 1.5, bar_y + 1.5], color="black", lw=0.6)
    axes.text(bar_x + 25, bar_y + 6, "this bar must measure 50.0 mm: print at Actual size / 100 %",
              ha="center", fontsize=8)
    axes.text(30, 20, "K1 jack pair, 1:1, TOP view (component side). Circles = coupon holes (slot + 0.4 mm),\n"
              "gray = 2.5 posts, dashed = bodies. Lay the jacks on it pins down: pins over the + marks.",
              fontsize=8, va="top")
    figure.savefig(pdf_path)
    print(pdf_path)


if __name__ == "__main__":
    main()
