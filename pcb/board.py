#!/usr/bin/env python3
"""Generate the open-oswst PCB."""

import json
import subprocess
from pathlib import Path

from kicad import Board

board = Board()
git_sha = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()

# Board revision: one per fab order, counting every JLCPCB order so far (A/B
# tests included), so it only ever goes up. Bump it when work on the next
# order starts, before anything else changes
REV = 10

# 70x59mm landscape, on A4 (297x210mm)
BX, BY = 113, 113  # board origin
BW, BH = 70, 59    # board size
# Rev 10 grew the board 9 mm at the bottom (south) edge, into the case's free
# strip above the bottom wall: 10 would leave 0.5 mm to the bottom lid posts.
# Everything else placed before that keeps its place in the case (the Heltec
# under its screen window): it measures from the old 50 mm outline
BH_REV9 = 50
INSET_LR = 5        # hole center distance from left/right edges
INSET_TB = 2.5      # hole center distance from top/bottom edges

board.add_rect(BX, BY, BW, BH)

# The bottom pair follows the bottom edge down (rev 10); the case's posts
# moved with them
for x in (BX + INSET_LR, BX + BW - INSET_LR):
    for y in (BY + INSET_TB, BY + BH - INSET_TB):
        board.add_mounting_hole(x, y, drill=2.2)

# Connectors along top edge — fit between drill holes (INSET_LR)
cy = BY + 4

# SD card (#44): the microSD breakout soldered straight on (0.1" pins),
# SDMMC 1-bit. Pads in the breakout's own pin order. D3 is tied to Ve here:
# high at the first command keeps the card in SD mode (low: SPI). D2, D1
# and CD go nowhere. Where BAT, SW and the Air Buddy's PA were (no amp
# since 2026-10-08), clear of the top-left mounting hole's screw head. Rev
# 10: one pin left, making room for the battery path beside it
board.add_header(BX + 23 - 2.54, cy, pins=9, label="SD",
                 pad_labels=["3V3", "GND", "DO", "CMD", "CLK", "D3", "D2", "D1", "CD"],
                 pad_nets=["Ve", "GND", "SD_D0", "SD_CMD", "SD_CLK", "Ve", None, None, None])

# The rest of the top edge (rev 10): SW and PTT have short cables, so they
# stay up here, and the battery path joins them: BAT -> SW -> HBAT side by
# side keeps VBAT and VSW short, never crossing the Heltec's header rows
# (0.5 mm power traces don't fit between their pins). Housings 6 mm wide,
# ~1.4 mm apart; PTT's ends ~1.5 mm short of the top-right screw head
# HBAT: feeds VSW out to Heltec's onboard SH1.25-2P LiPo input via an M2M pigtail.
# Heltec's battery input accepts 3.3-4.2V (per datasheet 3.2), so raw LiPo is correct here.
cx = BX + 36.3
JST2_STEP = 7.4  # one 2-pin housing (6 mm) and the gap to the next
board.add_jst_ph(cx, cy, pins=2, label="BAT",
                 pad_labels=["+", "-"], pad_nets=["VBAT", "GND"]); cx += JST2_STEP
board.add_jst_ph(cx, cy, pins=2, label="SW",
                 pad_nets=["VBAT", "VSW"]); cx += JST2_STEP
board.add_jst_ph(cx, cy, pins=2, label="HBAT",
                 pad_labels=["+", "-"], pad_nets=["VSW", "GND"]); cx += JST2_STEP
board.add_jst_ph(cx, cy, pins=2, label="PTT",
                 pad_nets=["PTT", "GND"])

# (CHNL encoder was on the top edge once, dropped: its pins GPIO7/GPIO5
# belong to the V4.3's RF front end. Channel is picked from the menu instead.)

# The volume knob on I2C (#44): a Qwiic knob through a JST-SH (Qwiic) to
# JST-PH adapter cable, pads in Qwiic order, so a straight-through cable
# fits. Mounted on the other side of the board, the order mirrors (3V3 and
# GND swap): check the cable against these labels before plugging the knob
# in. Rev 10: on the right edge (the Qwiic cable is long), between the
# Heltec's J2 row and the bottom-right mounting hole's screw head
board.add_jst_ph(BX + BW - 3.5, BY + 44.5, pins=4, label="VOL", angle=90,
                 pad_labels=["GND", "3V3", "SDA", "SCL"],
                 pad_nets=["GND", "Ve", "KNOB_SDA", "KNOB_SCL"])

# Heltec V4 headers — two 18-pin rows
J3_SPAN = 17 * 2.54  # 18 pins, 17 gaps
HELTEC_WIDTH = 22.86  # distance between J3 and J2 header rows
# J3 y (top row). Rev 10: 3 mm farther from the top-edge connectors (it was
# 4 mm above centre, now 1), giving their traces room. The case's screen
# window and USB-C hole follow it (models/case.py)
HY = BY + BH_REV9 / 2 - HELTEC_WIDTH / 2 - 1

board.add_header(BX + BW - 3 - J3_SPAN / 2, HY, pins=18, label="J3",
                 pad_labels=["GPIO7", "GPIO6", "GPIO5", "GPIO4", "GPIO3", "GPIO2",
                             "GPIO1", "GPIO38", "GPIO39", "GPIO40", "GPIO41", "GPIO42",
                             "GPIO45", "GPIO46", "GPIO37", "3V3a", "3V3b", "GND"],
                 # GPIO7 (FEM power), GPIO5 (FEM CTX), GPIO2 (FEM CSD) must stay
                 # unconnected: the Heltec V4.3 drives them. GPIO1 + GPIO37 are
                 # battery sense (ADC + divider enable). GPIO3/GPIO6 are the
                 # knob's I2C bus. GPIO45 (the wired encoder's switch once)
                 # is the SD card's CLK: a strapping pin, and CLK needs no
                 # pull-up, so nothing holds it high at reset.
                 pad_nets=[None, "KNOB_SCL", None, "MIC_OUT", "KNOB_SDA", None,
                           None, None, None, None, None, None,
                           "SD_CLK", None, None, None, None, "GND"])

# Heltec V4 J2 header (right side, pin 18→1 top to bottom)
J2_SPAN = J3_SPAN  # same 18 pins
board.add_header(BX + BW - 3 - J2_SPAN / 2, HY + HELTEC_WIDTH, pins=18, label="J2",
                 pad_labels=["GPIO19", "GPIO20", "GPIO21", "GPIO26", "GPIO48", "GPIO47",
                             "GPIO33", "GPIO34", "GPIO35", "GPIO36", "GPIO0", "RST",
                             "GPIO43", "GPIO44", "Ve_b", "Ve_a", "5V", "GND"],
                 # GPIO43/44: UART0 until the console moved to USB only; now
                 # the SD card's CMD and D0
                 pad_nets=[None, None, None, None, "LRC", "BCLK",
                           "DIN", None, None, None, "PTT", None,
                           "SD_CMD", "SD_D0", "Ve", "Ve", None, "GND"])

# Board headers along bottom edge
board.add_header(BX + BW / 2, BY + BH_REV9 - 9, pins=5, label="MIC",
                 pad_labels=["AR", "OUT", "GAIN", "VDD", "GND"],
                 pad_nets=[None, "MIC_OUT", None, "Ve", "GND"])
AMP_X = BX + 3 + 3 * 2.54
# Down by the bottom-left mounting hole (rev 10, clearing the top left for
# the Kenwood jacks): the header row stays 4.5 mm above the hole, room for
# the M2 screw head (3.8 mm), the breakout's edge past its pins, and 1 mm
AMP_Y = BY + BH - INSET_TB - 4.5
board.add_header(AMP_X, AMP_Y, pins=7, label="AMP", angle=180,
                 pad_labels=["Vin", "GND", "SD", "GAIN", "DIN", "BCLK", "LRC"],
                 pad_nets=["Ve", "GND", None, None, "DIN", "BCLK", "LRC"])
board.add_header(AMP_X, AMP_Y - 12.954, pins=2, label="AMP_SPK", angle=180,
                 pitch=3.5, pad_labels=["+", "-"], pad_nets=["SPK+", "SPK-"])

# SPK connector (rotated 90°), just right of the amp breakout, beside its
# AMP_SPK terminals (rev 10: the left edge went to the Kenwood jacks)
board.add_jst_ph(BX + 24, BY + 46, pins=2, label="SPK", angle=90,
                 pad_labels=["-", "+"], pad_nets=["SPK-", "SPK+"])

# Kenwood K1 jacks (rev 10): the homemade breakout (pcb/k1/board.py) copied
# onto this board, to race it: the same measured footprints
# (pcb/k1/k1_jacks.json, top view, origin = the 3.5 bore's nose tip, bores
# pointing -y) and the same 10-pin header map, every jack pin to its own
# header pin and nothing else. Which pin is tip/ring/sleeve/switch gets
# mapped on the bench. Bores out the left edge, and MIRRORED (user,
# 2026-10-19): the frame's (x, y) lands at (K1_X + y, K1_Y + x), so the 2.5
# is below the 3.5 and the jacks mount from the BACK (B side) of this
# view. As on the homemade board, the jack faces sit at the board edge and
# the noses stick out past it
K1 = json.loads((Path(__file__).parent / "k1/k1_jacks.json").read_text())
K1_X = BX - 2.5   # nose tips 2.5 mm out past the left edge, as on the homemade board
K1_Y = BY + 16    # header clear of SD above, 2.5 body clear of the amp below
K1_PIN_35 = {"size": 2.6, "drill": 1.7}   # 1.32 mm tabs; 1.7 fit the test plate
K1_PIN_25 = {"size": 1.45, "drill": 1.1}  # 0.76 mm pins, only 1.9-2.0 mm apart
K1_POST_DRILL = 1.7                       # the 2.5's plastic posts: holes, no copper
K1_HEADER = {"size": 1.7, "drill": 1.0}
# Header pin -> (jack, jack pin), as on the homemade board
K1_HEADER_MAP = [("3.5", 2), ("3.5", 1), ("3.5", 3), ("3.5", 4), ("3.5", 5),
                 ("2.5", 2), ("2.5", 1), ("2.5", 3), ("2.5", 4), ("2.5", 5)]
K1_HEADER_Y = 21.0       # in the jack frame, from the homemade board
K1_HEADER_FIRST_X = -6.43


def k1_at(x_mm, y_mm):
    return (round(K1_X + y_mm, 3), round(K1_Y + x_mm, 3))


def k1_net(jack, number):
    return f"K1_{jack.replace('.', '')}_{number}"


for jack, pad_style in (("3.5", K1_PIN_35), ("2.5", K1_PIN_25)):
    pads = []
    for number, pin in enumerate(K1[jack]["pins"], 1):
        x_mm, y_mm = k1_at(pin["x"], pin["y"])
        pads.append({"name": str(number), "x": x_mm, "y": y_mm, **pad_style,
                     "net": k1_net(jack, number)})
    for post in K1[jack].get("posts", []):
        x_mm, y_mm = k1_at(*post)
        pads.append({"name": "", "x": x_mm, "y": y_mm, "drill": K1_POST_DRILL, "plated": False})
    board.add_footprint(f"K1_Jack_{jack}mm_measured", 0, 0, pads, label=f"K1_{jack}")

k1_header = []
for header, (jack, number) in enumerate(K1_HEADER_MAP, 1):
    x_mm, y_mm = k1_at(K1_HEADER_FIRST_X + (header - 1) * 2.54, K1_HEADER_Y)
    k1_header.append({"name": str(header), "x": x_mm, "y": y_mm, **K1_HEADER,
                      "net": k1_net(jack, number)})
board.add_footprint("PinHeader_1x10_P2.54mm", 0, 0, k1_header, label="K1")

# Labels on both silkscreens: the jacks go in from the back, the header may be
# soldered from either side. Pin 1 and 10 beside the header's ends (pin 1 is
# the top end), each jack's size beside its body
for layer in ("F.SilkS", "B.SilkS"):
    for header in (1, 10):
        x_mm, y_mm = k1_at(K1_HEADER_FIRST_X + (header - 1) * 2.54, K1_HEADER_Y)
        board.add_text(round(x_mm + 2.2, 3), y_mm, str(header), layer=layer, size=0.8, thickness=0.12)
    for jack in ("3.5", "2.5"):
        bore_x = K1[jack]["bore"][0][0]
        x_mm, y_mm = k1_at(bore_x, 15.0)
        board.add_text(x_mm, y_mm, jack, layer=layer, size=0.8, thickness=0.12)

# Silkscreen — project name, revision and git SHA
board.add_text(BX + BW / 2, BY + BH - 2, f"open-oswst  rev{REV}  {git_sha}", size=0.8, thickness=0.12)

# Ground plane — GND zone covering full board, both copper layers
board.add_zone("GND", [
    (BX, BY), (BX + BW, BY), (BX + BW, BY + BH), (BX, BY + BH),
], clearance=0.25, thermal_gap=0.3, thermal_bridge_width=0.3,
   # Every island goes: a scrap of fill the traces cut off is copper
   # connected to nothing (DRC: "isolated copper fill", 2026-10-10)
   island_removal_mode=0)

# GND stitching vias across the open middle, under the Heltec (between the
# J3 and J2 rows): wherever the traces cut one layer's fill into an island,
# a via ties it to the other layer's ground, so it stays and the plane under
# the SD lines (20-40 MHz on a real board) has no hole. 2026-10-10 the SD
# DO trace boxed in ~20x25 mm of F.Cu here. Freerouting routes around them
for x in (BX + 37, BX + 45, BX + 53, BX + 61):
    for y in (HY + 7, HY + 15):
        board.add_via(x, y, "GND")

board.save("open-oswst.kicad_pcb")
