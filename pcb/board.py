#!/usr/bin/env python3
"""Generate the open-oswst PCB."""

import subprocess
from kicad import Board

board = Board()
git_sha = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()

# 70x50mm landscape, centered on A4 (297x210mm)
BX, BY = 113, 113  # board origin
BW, BH = 70, 50    # board size
INSET_LR = 5        # hole center distance from left/right edges
INSET_TB = 2.5      # hole center distance from top/bottom edges

board.add_rect(BX, BY, BW, BH)

for x in (BX + INSET_LR, BX + BW - INSET_LR):
    for y in (BY + INSET_TB, BY + BH - INSET_TB):
        board.add_mounting_hole(x, y, drill=2.2)

# Connectors along top edge — fit between drill holes (INSET_LR)
cy = BY + 4

# SD card (#44): the microSD breakout soldered straight on (0.1" pins),
# SDMMC 1-bit. Pads in the breakout's own pin order. D3 is tied to Ve here:
# high at the first command keeps the card in SD mode (low: SPI). D2, D1
# and CD go nowhere. Where BAT, SW and the Air Buddy's PA were (no amp
# since 2026-10-08), clear of the top-left mounting hole's screw head
board.add_header(BX + 23, cy, pins=9, label="SD",
                 pad_labels=["3V3", "GND", "DO", "CMD", "CLK", "D3", "D2", "D1", "CD"],
                 pad_nets=["Ve", "GND", "SD_D0", "SD_CMD", "SD_CLK", "Ve", None, None, None])

# (CHNL encoder was here, dropped: its pins GPIO7/GPIO5 belong to the V4.3's
# RF front end. Channel is picked from the menu instead. Gap kept so VOL/PTT
# don't move.)
cx = BX + 47

# The volume knob on I2C (#44): a Qwiic knob through a JST-SH (Qwiic) to
# JST-PH adapter cable. Same place and footprint as the wired encoder's
# connector; pads in Qwiic order, so a straight-through cable fits. Mounted
# on the other side of the board, the order mirrors (3V3 and GND swap):
# check the cable against these labels before plugging the knob in
board.add_jst_ph(cx, cy, pins=4, label="VOL",
                 pad_labels=["GND", "3V3", "SDA", "SCL"],
                 pad_nets=["GND", "Ve", "KNOB_SDA", "KNOB_SCL"]); cx += 10
board.add_jst_ph(cx, cy, pins=2, label="PTT",
                 pad_nets=["PTT", "GND"])

# HBAT: feeds VSW out to Heltec's onboard SH1.25-2P LiPo input via an M2M pigtail.
# Heltec's battery input accepts 3.3-4.2V (per datasheet 3.2), so raw LiPo is correct here.
# Placed above SPK so VSW from BAT/SW (top of board) doesn't cross the SPK+/SPK- traces.
# BAT and SW: a second column beside HBAT and SPK (moved off the top edge for
# the SD card). Battery -> BAT -> SW (VSW) -> HBAT, all in one corner
board.add_jst_ph(BX + 12, BY + INSET_TB + 7, pins=2, label="BAT", angle=90,
                 pad_labels=["+", "-"], pad_nets=["VBAT", "GND"])
board.add_jst_ph(BX + 12, BY + INSET_TB + 14, pins=2, label="SW", angle=90,
                 pad_nets=["VBAT", "VSW"])

board.add_jst_ph(BX + 5, BY + INSET_TB + 7, pins=2, label="HBAT", angle=90,
                 pad_labels=["+", "-"], pad_nets=["VSW", "GND"])

# SPK connector on left edge (rotated 90°, routes down to AMP_SPK)
board.add_jst_ph(BX + 5, BY + INSET_TB + 14, pins=2, label="SPK", angle=90,
                 pad_labels=["-", "+"], pad_nets=["SPK-", "SPK+"])

# Heltec V4 headers — two 18-pin rows
J3_SPAN = 17 * 2.54  # 18 pins, 17 gaps
HELTEC_WIDTH = 22.86  # distance between J3 and J2 header rows
HY = BY + BH / 2 - HELTEC_WIDTH / 2 - 4  # J3 y (top row), shifted 4mm toward top-edge JSTs

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
board.add_header(BX + BW / 2, BY + BH - 9, pins=5, label="MIC",
                 pad_labels=["AR", "OUT", "GAIN", "VDD", "GND"],
                 pad_nets=[None, "MIC_OUT", None, "Ve", "GND"])
AMP_X = BX + 3 + 3 * 2.54
AMP_Y = BY + BH - 3 - 5 - 4
board.add_header(AMP_X, AMP_Y, pins=7, label="AMP", angle=180,
                 pad_labels=["Vin", "GND", "SD", "GAIN", "DIN", "BCLK", "LRC"],
                 pad_nets=["Ve", "GND", None, None, "DIN", "BCLK", "LRC"])
board.add_header(AMP_X, AMP_Y - 12.954, pins=2, label="AMP_SPK", angle=180,
                 pitch=3.5, pad_labels=["+", "-"], pad_nets=["SPK+", "SPK-"])

# Silkscreen — project name and git SHA
board.add_text(BX + BW / 2, BY + BH - 2, f"open-oswst  {git_sha}", size=0.8, thickness=0.12)

# Ground plane — GND zone covering full board, both copper layers
board.add_zone("GND", [
    (BX, BY), (BX + BW, BY), (BX + BW, BY + BH), (BX, BY + BH),
], clearance=0.25, thermal_gap=0.3, thermal_bridge_width=0.3,
   # Every island goes: a scrap of fill the traces cut off is copper
   # connected to nothing (DRC: "isolated copper fill", 2026-10-10)
   island_removal_mode=0)

board.save("open-oswst.kicad_pcb")
