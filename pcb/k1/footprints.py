#!/usr/bin/env python3
"""The K1 jack pair's footprints, measured: writes k1_jacks.json.

    footprints.py [k1_jacks.json]

Both jacks were plugged into a K1 plug and stabbed through graph paper
together (2026-10-16), so the spacing between them is the plug's. Two photos:

- Backlit holes, measured with measure.py --red (the user marked each real
  hole with a red bar). The pins came through the back of the paper, so the
  photo shows the pin-tip side: a BOTTOM view.
- A close-up of both jacks lying pins up (also a bottom view), for what
  didn't pierce the paper: the 2.5's plastic posts, the bodies, the bores.
  It's fitted onto the holes; it fits without a mirror (0.18 mm rms) and
  badly with one, which confirms the holes photo is a bottom view.

Output frame: TOP view (component side, as KiCad draws it), mm, origin =
the 3.5 bore at its nose tip, bores pointing -y (the plug comes in from -y).
"""

import json
import sys

import numpy as np

# Backlit holes: grid mm, bottom view (measure.py --red output)
HOLES_35 = [(13.675, 3.125), (7.142, 3.622), (3.344, 8.471), (10.313, 10.350), (11.709, 13.340)]
SLOTS_35 = [(1.13, 92.6), (1.74, 175.1), (1.17, 89.3), (1.22, 172.4), (1.32, 166.1)]  # length, deg
HOLES_25 = [(6.601, 15.986), (4.638, 16.475), (3.980, 18.166), (6.193, 23.376), (4.316, 23.706)]
SLOTS_25 = [(0.51, 88.0), (0.82, 178.1), (0.90, 2.2), (0.92, 5.5), (0.65, 173.3)]

# Close-up, pixels (crop at 860,1120 of PXL_20261016_190349894), in the same
# order as the holes they made (matched by the best fit)
TABS_35_PX = [(75, 820), (330, 825), (495, 645), (228, 555), (190, 445)]
PINS_25_PX = [(390, 350), (462, 355), (492, 290), (420, 75), (490, 73)]
POSTS_25_PX = [(310, 145), (455, 145), (424, 290)]
BODY_35_PX = [(18, 425), (555, 425), (555, 865), (18, 865)]
BODY_25_PX = [(235, 35), (595, 35), (595, 425), (235, 425)]
# Each bore: from the body's back to the nose tip, through the nose's centre
BORE_35_PX = [(18, 643), (690, 643)]
BORE_25_PX = [(235, 230), (700, 230)]


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else "pcb/k1/k1_jacks.json"
    holes = np.array(HOLES_35 + HOLES_25)

    # One similarity transform for the whole close-up: one photo, one plug
    close_up = np.array(TABS_35_PX + PINS_25_PX, dtype=float)
    to_holes = similarity(close_up, holes)
    error = to_holes(close_up) - holes
    print(f"close-up fitted onto the holes: {np.sqrt((error ** 2).sum(axis=1).mean()):.2f} mm rms")

    parts = {
        "holes_35": np.array(HOLES_35), "holes_25": np.array(HOLES_25),
        "posts_25": to_holes(POSTS_25_PX),
        "body_35": to_holes(BODY_35_PX), "body_25": to_holes(BODY_25_PX),
        "bore_35": to_holes(BORE_35_PX), "bore_25": to_holes(BORE_25_PX),
    }

    # Turn so the bores point -y (the plug holds them parallel: average them),
    # then mirror x: bottom view -> top view
    direction = unit(parts["bore_35"][1] - parts["bore_35"][0]) + unit(parts["bore_25"][1] - parts["bore_25"][0])
    turn = -np.pi / 2 - np.arctan2(direction[1], direction[0])
    rotation = np.array([[np.cos(turn), -np.sin(turn)], [np.sin(turn), np.cos(turn)]])
    for name, points in parts.items():
        parts[name] = (np.asarray(points, dtype=float) @ rotation.T) * [-1, 1]
    origin = parts["bore_35"][1].copy()
    for name in parts:
        parts[name] = parts[name] - origin

    # The user, who has the parts: on the 2.5, the two pins nearest the bore's
    # back share one across-bore position, the two nearest the nose another
    # (of those, the one at -x measured truer)
    holes_25 = parts["holes_25"]
    holes_25[[0, 1], 0] = holes_25[[0, 1], 0].mean()
    holes_25[[3, 4], 0] = 0.25 * holes_25[3, 0] + 0.75 * holes_25[4, 0]

    out = {"units": "mm, TOP view, origin = 3.5 bore at its nose tip, bores point -y"}
    for jack, key, slots in (("3.5", "35", SLOTS_35), ("2.5", "25", SLOTS_25)):
        # The user: every pin is at 0 or 90 degrees, and all of a jack's pins
        # are the same size: snap the angles, one slot length per jack
        slot_mm = round(float(np.mean([length for length, _ in slots])), 2)
        pins = []
        for (x_mm, y_mm), (_, angle) in zip(parts[f"holes_{key}"], slots):
            # Turned with the frame, then mirrored: angle -> 180 - (angle + turn)
            top_view = (180 - (angle + np.degrees(turn))) % 180
            snapped = 0 if min(top_view, 180 - top_view) < 45 else 90
            pins.append({"x": round(float(x_mm), 2), "y": round(float(y_mm), 2), "slot_mm": slot_mm, "slot_deg": snapped})
        out[jack] = {
            "pins": pins,
            "body": np.round(parts[f"body_{key}"], 2).tolist(),
            "bore": np.round(parts[f"bore_{key}"], 2).tolist(),
        }
    out["2.5"]["posts"] = np.round(parts["posts_25"], 2).tolist()
    out["bore_gap_mm"] = round(float(parts["bore_25"][1][0] - parts["bore_35"][1][0]), 2)

    with open(out_path, "w") as handle:
        json.dump(out, handle, indent=1)
    for jack in ("3.5", "2.5"):
        print(f"{jack}: slots {out[jack]['pins'][0]['slot_mm']} mm, pins",
              [(pin["x"], pin["y"], pin["slot_deg"]) for pin in out[jack]["pins"]])
    print(f"bore gap {out['bore_gap_mm']} mm -> {out_path}")


def similarity(source, target):
    """Best rotation + scale + shift (no mirror) taking source onto target"""
    source, target = np.asarray(source, dtype=float), np.asarray(target, dtype=float)
    source_centre, target_centre = source.mean(axis=0), target.mean(axis=0)
    centred_source, centred_target = source - source_centre, target - target_centre
    left, singular, right = np.linalg.svd(centred_source.T @ centred_target)
    rotation = left @ right
    scale = singular.sum() / (centred_source ** 2).sum()
    return lambda points: (np.asarray(points, dtype=float) - source_centre) @ rotation * scale + target_centre


def unit(vector):
    return vector / np.linalg.norm(vector)


if __name__ == "__main__":
    main()
