#!/usr/bin/env python3
"""Generate the open-oswst PCB."""

import json
import subprocess
from pathlib import Path

from kicad import Board

board = Board()
git_sha = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()

# Board revision, MAJOR.MINOR: the major goes up only when firmware for the
# last board can't run this one (pins or drivers change); a compatible order
# bumps the minor. Set it when work on the next order starts, before anything
# else changes
REV = "10.2"

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

# The rest of the top edge (rev 10): SW has a short cable, so it stays up
# here, and the battery path joins it: BAT -> SW -> HBAT side by side keeps
# VBAT and VSW short, never crossing the Heltec's header rows (0.5 mm power
# traces don't fit between their pins). Housings 6 mm wide, ~1.4 mm apart;
# HBAT's end ~1.6 mm short of the top-right screw head. Rev 10.1: SPK (a
# short cable too) comes up here beside SD, where PTT's move made room
# HBAT: feeds VSW out to Heltec's onboard SH1.25-2P LiPo input via an M2M pigtail.
# Heltec's battery input accepts 3.3-4.2V (per datasheet 3.2), so raw LiPo is correct here.
cx = BX + 36.3
JST2_STEP = 7.4  # one 2-pin housing (6 mm) and the gap to the next
cx += JST2_STEP  # rev 10.1's SPK slot, empty since rev 10.2 (beside PTT now)
board.add_jst_ph(cx, cy, pins=2, label="BAT",
                 pad_labels=["+", "-"], pad_nets=["VBAT", "GND"]); cx += JST2_STEP
board.add_jst_ph(cx, cy, pins=2, label="SW",
                 pad_nets=["VBAT", "VSW"]); cx += JST2_STEP
board.add_jst_ph(cx, cy, pins=2, label="HBAT",
                 pad_labels=["+", "-"], pad_nets=["VSW", "GND"]); cx += JST2_STEP

# PTT (rev 10.1): top-left corner, under the screw, beside the K1 jack whose
# sleeve is wired to it. At the top right, the PTT net ran the whole top
# edge from the jack, an RF pickup, and freerouting then found no way round
# for VBAT. Horizontal: the 5.7 mm between the screw head and the 3.5 jack's
# body is too short for the 6 mm housing standing along the edge. The
# button is in the right wall, so its cable is long now (~60 mm)
PTT_X = BX + 2.5
PTT_Y = BY + 7.3
board.add_jst_ph(PTT_X, PTT_Y, pins=2, label="PTT",
                 pad_nets=["PTT", "GND"])
# The internal speaker (rev 10.2), beside PTT: it hangs off the K1 2.5 jack's
# tip switch below. Up on the top edge beside SD the SD bus and the PCM's
# traces boxed it in. Its cable has to reach here (user, 2026-11-08: the
# newer speakers' cables are longer)
board.add_jst_ph(PTT_X + JST2_STEP, PTT_Y, pins=2, label="SPK",
                 pad_labels=["-", "+"], pad_nets=["GND", "SPK+"],
                 ref_at=(0, 2.4))  # its name below: copper above it

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
                 # GPIO4: the mic, through the anti-alias RC (R3, C2)
                 pad_nets=[None, "KNOB_SCL", None, "ADC", "KNOB_SDA", None,
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
                           "SD_CMD", "SD_D0", "Ve", "Ve", None, "GND"],
                 ref_at=(-12, 2.5))  # below the row, west: the PCM's outline is above it

# The Heltec's outline on the floor side, where it hangs on its sockets, so
# nothing tall lands under it. Measured (micrometer and a marked-up pin map,
# 2026-11-06): 51.73 x 25.47, the USB end's edge 3.97 mm past pin 1 (east,
# J3's GND/3V3 end), full width to 1.03 mm past pin 18, then 45-degree
# chamfers to the antenna tip. Its USB end overhangs our east edge by ~1 mm:
# the outline stops short of the edge there
HELTEC_LENGTH = 51.73
HELTEC_BODY_WIDTH = 25.47
HELTEC_USB_PAST_PIN1 = 3.97
HELTEC_FULL_WIDTH_PAST_PIN18 = 1.03
heltec_pin1_x = BX + BW - 3
heltec_usb_x = min(heltec_pin1_x + HELTEC_USB_PAST_PIN1, BX + BW - 0.3)
heltec_chamfer_x = heltec_pin1_x - J3_SPAN - HELTEC_FULL_WIDTH_PAST_PIN18
heltec_tip_x = heltec_pin1_x + HELTEC_USB_PAST_PIN1 - HELTEC_LENGTH
heltec_chamfer = heltec_chamfer_x - heltec_tip_x  # 45 degrees: as deep as long
heltec_top = HY + HELTEC_WIDTH / 2 - HELTEC_BODY_WIDTH / 2
heltec_bottom = HY + HELTEC_WIDTH / 2 + HELTEC_BODY_WIDTH / 2
board.add_outline([(heltec_usb_x, heltec_top), (heltec_chamfer_x, heltec_top),
                   (heltec_tip_x, heltec_top + heltec_chamfer),
                   (heltec_tip_x, heltec_bottom - heltec_chamfer),
                   (heltec_chamfer_x, heltec_bottom), (heltec_usb_x, heltec_bottom)],
                  layer="B.SilkS", width=0.12)

# Rev 10.2: the PCM5102A DAC board (purple "GY-PCM5102"), the speaker
# path's I2S end, where rev 10.1 kept AMP2's room: against the east edge
# between the Heltec's header rows, centred between the rows, on the lid
# side face up with the other daughter boards (user, 2026-11-09, after the
# coupon fit). 31.46 x 16.60 mm, 6.23 mm tall with its 3.5 mm jack (calipers).
# The jack stays soldered on, but the plastic it had sticking 3.6 mm past
# one long edge is clipped off (user, 2026-11-08): no plug fits it in the
# case anyway. Holes off a derotated photo (OpenCV, scaled by their 2.54
# pitch): a 6-pin I2S column 1.1 mm in from its short edge, a 9-hole row
# 1.44 mm in from the long edge away from the jack.
# PCM frame: as printed, jack top-left, I2S column at the right, the 9-hole
# row along the bottom. Here it lies turned half way round: the I2S column
# west, by J2's GPIO33/47/48; the 9-hole row north, the jack south-east
PCM_W = 31.46
PCM_H = 16.60
PCM_RIGHT = BX + BW - 0.3
PCM_BOTTOM = HY + HELTEC_WIDTH / 2 + PCM_H / 2
PCM_LEFT = PCM_RIGHT - PCM_W
PCM_TOP = PCM_BOTTOM - PCM_H


def pcm_at(x_mm, y_mm):
    """A point in the PCM's printed frame, turned half way round onto this view."""
    return PCM_RIGHT - x_mm, PCM_BOTTOM - y_mm


PCM_I2S = [("VIN", "Ve"), ("GND", "GND"), ("LCK", "LRC"), ("DIN", "DIN"), ("BCK", "BCLK"),
           ("SCK", "GND")]  # SCK grounded: its PLL clocks off BCK
# The 9-hole row from the jack end: line out (L only: the firmware sends mono
# to both slots), its analog 3.3 V (unmarked), then the mode pins. XSMT must
# be HIGH (low = muted: the bench board was silent until it was); FLT, DEMP,
# FMT low.
# Named as its own silkscreen marks them: 4 FMT, 3 XSMT, 2 DEMP, 1 FLT. Its G
# holes are its own ground, which reaches ours through the I2S column's GND
PCM_ROW = [("L", "AMP_IN"), ("G", None), ("R", None), ("G", None), ("", None),
           ("4", "GND"), ("3", "Ve"), ("2", "GND"), ("1", "GND")]
pcm_pads = []
for number, (name, net) in enumerate(PCM_I2S):
    x_mm, y_mm = pcm_at(PCM_W - 1.1, PCM_H - 2.02 - (5 - number) * 2.54)
    pcm_pads.append((name, x_mm, y_mm, net))
for number, (name, net) in enumerate(PCM_ROW):
    x_mm, y_mm = pcm_at(3.32 + number * 2.54, PCM_H - 1.44)
    pcm_pads.append((name, x_mm, y_mm, net))
# The pins only tied to ground (SCK and the mode pins) join the ground fill
# solidly: the I2S traces fanning past the 9-hole row kept boxing them in to
# one thermal spoke (DRC: starved thermal, 2026-11-08). They carry no
# current; their joints just take a little more heat. Its GND pin, the
# supply return, keeps its spokes
PCM_TIES = {"SCK", "4", "2", "1"}
board.add_footprint("PCM5102A_GY", 0, 0,
                    [{"name": str(number + 1), "x": x_mm, "y": y_mm, "size": 1.7, "drill": 1.0,
                      "net": net, "solid": name in PCM_TIES}
                     for number, (name, x_mm, y_mm, net) in enumerate(pcm_pads)],
                    label="PCM")
board.add_rect(PCM_LEFT, PCM_TOP, PCM_W, PCM_H, layer="F.SilkS", width=0.12)
for name, x_mm, y_mm, _ in pcm_pads:
    in_row = y_mm < PCM_TOP + 2
    board.add_text(x_mm + (0 if in_row else 2.4), y_mm + (1.6 if in_row else 0), name,
                   layer="F.SilkS", size=0.8, thickness=0.12)
board.add_text(PCM_LEFT + PCM_W / 2, PCM_BOTTOM - 6, "PCM5102A", layer="F.SilkS", size=1,
               thickness=0.15)

# MAX9814 mic amp (Amazon clone of Adafruit 1713), turned (rev 10.1): its
# header runs north-south (AR south) and the board lies flat over this one
# on the lid side, reaching west from its header, so the flying leads land
# near the K1 jacks: the ring, a high impedance millivolt line, stays short.
# Its own capsule is gone: the clone's two capsule pads come down on short
# flying leads to MIC_IN below, and the mic is the K1 jack's ring (or the
# board's capsule through the ring's NC switch). Clone layout (measured off
# a marked-up photo 2026-11-06, the capsule pads checked with the real
# board on a printed coupon, models/mic_coupon.py): from its header, the
# capsule pads (Q1) 18.7 mm on, centred on GAIN, 2.54 apart; its far edge
# 23.5 mm on, its near edge 1.6 mm behind; 13.8 mm wide
MIC_Q1 = 18.7
MIC_FAR_EDGE = 23.5
MIC_NEAR_EDGE = 1.6
MIC_WIDTH = 13.8
# Rev 10.2: along the south edge, west to east, the mic amp board (lid side),
# the capsule (floor side) and the LM386 (lid side, drawn further down). The capsule
# sits between the two amps, centred in the gap between the mic amp's
# header pads and the LM386's west edge (~1.9 mm each side of its can): out
# of the corner, for the sound (user, 2026-11-08), at the cost of a longer
# high-impedance CAPSULE trace from the K1 3.5 jack's ring switch
LM386_W = 21.04
LM386_H = 12.78
LM386_LEFT = BX + BW - 3.5 - 5.0 - LM386_W  # 5 mm west of VOL's pins: its housing, its name
LM386_TOP = BY + BH - 4.8 - LM386_H         # the MAX98357A's south edge, north of the screw
MIC_X = BX + 26.0  # its outline's west end ~2.5 mm in from the edge
MIC_Y = BY + 47
CAPSULE_DIAMETER = 9.74  # micrometer, 2026-11-06
CAPSULE_X = (MIC_X + 0.85 + LM386_LEFT) / 2
# CAPSULE turned 270 degrees: pin 1 north (frame (0, -y) = y mm east,
# (x, 0) = x mm south); MIC_IN 90, like the amp's header: pin 1 south
# (frame (0, y) = y mm east). Names clear of pads and outlines
# GAIN to VDD = 40 dB: 60 dB (floating) swung rail to rail on the bench, 50
# clipped the Disco32. GND is the mic ground (JP1 joins it to the board's)
# Its header turned the other way from MIC_IN and CAPSULE: AR south, as the
# real board's pins land when it lies reaching west (user, 2026-11-06)
board.add_header(MIC_X, MIC_Y, pins=5, label="MIC", angle=90,
                 pad_labels=["AR", "OUT", "GAIN", "VDD", "GND"],
                 pad_nets=[None, "MIC_OUT", "Ve", "Ve", "MIC_GND"], ref_at=(0, -4.0))
# Flying-lead holes, right under the clone's old capsule pads. "+" is the
# amp's input (MICBIAS through its 2.2k, into MICIN through 0.1 uF); "-" is
# the amp board's ground, which on the bench had to be the ground of every
# mic part (the bias filter cap's above all: anywhere else put the Heltec
# USB frames' 25 Hz into the ring, 2026-11-05)
# Turned with the amp board's header (user, 2026-11-06): "+" south. Which
# Q1 pad is the input gets checked at assembly: it idles at the amp's
# MICBIAS (~2 V through 2.2k), the other is its ground
board.add_header(MIC_X - MIC_Q1, MIC_Y, pins=2, label="MIC_IN", angle=90,
                 pad_labels=["+", "-"], pad_nets=["MIC_IN", "MIC_GND"], ref_at=(0, 4.5))
# The board's own mic: the 9x7 mm electret capsule, on the K1 3.5 jack's
# ring NC switch (net CAPSULE, K1_NETS): plugging a headset in opens the
# switch and the headset's mic, on the ring, takes over. Between the two amp
# boards; it faces the floor, so the case's mic hole follows it
# On the floor side, 1 mm north of the mic amp's row (rev 10.1's fit)
CAPSULE_Y = MIC_Y - 1.0
CAPSULE_PITCH = 3.1  # its legs, off a marked-up photo (3.1-3.2 mm, 2026-11-06)
board.add_circle(CAPSULE_X, CAPSULE_Y, CAPSULE_DIAMETER, layer="B.SilkS", width=0.12)
board.add_header(CAPSULE_X, CAPSULE_Y, pins=2, label="CAPSULE", angle=270, pitch=CAPSULE_PITCH,
                 pad_labels=["+", "-"], pad_nets=["CAPSULE", "MIC_GND"], ref_at=(6.5, 0),
                 ref_layer="B.SilkS")  # its name on its own side, clear of both outlines
# Which hole is which, on both sides (the capsule faces the floor, the amp
# board's leads come from the lid side): "+" (pin 1) north
for layer in ("F.SilkS", "B.SilkS"):
    # MIC_IN's marks east of it, "+" south; the capsule's west (east is the
    # amp board's edge), "+" north
    for x_mm, y_mm, half_pitch, plus_south in ((MIC_X - MIC_Q1 + 1.8, MIC_Y, 1.27, True),
                                               (CAPSULE_X - 1.8, CAPSULE_Y, CAPSULE_PITCH / 2,
                                                False)):
        plus_dy = half_pitch if plus_south else -half_pitch
        board.add_text(x_mm, y_mm + plus_dy, "+", layer=layer, size=0.8, thickness=0.12)
        board.add_text(x_mm, y_mm - plus_dy, "-", layer=layer, size=0.8, thickness=0.12)
# The amp board's outline, so nothing else lands under it
board.add_rect(MIC_X - MIC_FAR_EDGE, MIC_Y - MIC_WIDTH / 2, MIC_FAR_EDGE + MIC_NEAR_EDGE,
               MIC_WIDTH, layer="F.SilkS", width=0.12)
# Rev 10.2 (WIP): the MAX98357A is gone (its BTL output was the harsh sound,
# 2026-11-07). The speaker amp is now the green mini LM386 module
# (GODIYMODULES "Adjustable Mini LM386 3-12V"), on the lid side, face up, in
# the south-east corner, just west of VOL (user, 2026-11-08): its volume
# trimmer gets set at assembly and stays reachable with the lid off, and it
# fit there on the printed coupon. 21.04 x
# 12.78 mm, 7.26 mm tall at its 220 uF cap (calipers, user 2026-11-08). Six
# pads in three pairs, 2.54 apart within a pair; where the pairs sit came
# off a bench photo scaled to the calipers' outline (+-0.3 mm): a coupon
# before fab. Gain 20 as shipped (its "ADD-dB" pads, pins 1-8, are empty).
# Turned 180 degrees from its silkscreen's reading: IN/GND west, OUT north,
# the supply pads south. Its silkscreen calls both
# output pads OUT: the one by the 220 uF cap is the signal (OUT+, to the K1
# 2.5 tip), the other is ground (bench, 2026-11-08). Its supply is Ve, the
# Heltec's 3.3 V, like the mic amp beside it and the MAX98357A before it:
# it ran clean from the Heltec on the bench, the loudest level usable with
# its trimmer set at assembly (user, 2026-11-08); the battery's 3.4-4.2 V
# would have bought ~1-2.5 dB but a 0.5 mm power trace round the Heltec.
# Its three ground pads are one node on the module. IN waits for the
# PCM5102A (AMP_IN)
LM386_PADS = [("GND", 2.06, 2.02, "GND"), ("IN", 2.06, 4.56, "AMP_IN"),      # west column
              ("OUT-", 8.05, 1.61, "GND"), ("OUT+", 10.59, 1.61, "AMP_OUT"),  # north row
              ("-", 8.05, 10.79, "GND"), ("+", 10.59, 10.79, "Ve")]          # south row: supply
board.add_footprint("LM386_mini", LM386_LEFT, LM386_TOP,
                    [{"name": str(number + 1), "x": x_mm, "y": y_mm, "size": 1.7,
                      "drill": 1.0, "net": net}
                     for number, (_, x_mm, y_mm, net) in enumerate(LM386_PADS)],
                    label="LM386")
board.add_rect(LM386_LEFT, LM386_TOP, LM386_W, LM386_H, layer="F.SilkS", width=0.12)
for name, x_mm, y_mm, _ in LM386_PADS:
    board.add_text(LM386_LEFT + x_mm + (2.4 if x_mm < 5 else 0),
                   LM386_TOP + y_mm + (0 if x_mm < 5 else (1.6 if y_mm < 5 else -1.6)),
                   name.removeprefix("OUT"), layer="F.SilkS", size=0.8, thickness=0.12)
board.add_text(LM386_LEFT + 9.32, LM386_TOP + 4.8, "OUT", layer="F.SilkS", size=0.8,
               thickness=0.12)
board.add_text(LM386_LEFT + LM386_W / 2 + 3, LM386_TOP + LM386_H / 2, "LM386",
               layer="F.SilkS", size=1, thickness=0.15)


# Kenwood K1 jacks (rev 10): the homemade breakout (pcb/k1/board.py) copied
# onto this board, to race it: the same measured footprints
# (pcb/k1/k1_jacks.json, top view, origin = the 3.5 bore's nose tip, bores
# pointing -y). Rev 10 broke every jack pin out to a 10-pin header for the
# bench mapping; rev 11 wires the mic pins in (K1_NETS). Bores out the left
# edge, and MIRRORED (user,
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


def k1_at(x_mm, y_mm):
    return (round(K1_X + y_mm, 3), round(K1_Y + x_mm, 3))


# The jack pins the mic half uses (rev 11), by (jack, pin) from the bench map
# (rev 10 header pin -> function, 2026-10-27/11-05): 3.5 sleeve = PTT to
# ground, 3.5 ring = the headset mic, 3.5 ring's NC switch = the board's
# capsule (open with a plug in), 2.5 sleeve = the headset mic's ground.
# Rev 10.2, the speaker half: 2.5 tip = the LM386's output (the headset's
# earpiece), and the 2.5 tip's NC switch = the internal speaker (SPK), cut
# off with a plug in. The earpiece returns on the 2.5 sleeve (MIC_GND, as
# the K1 standard shares it): fine, the radio never talks and listens at once
K1_NETS = {("3.5", 3): "PTT", ("3.5", 4): "CAPSULE", ("3.5", 5): "MIC_IN",
           ("2.5", 1): "SPK+", ("2.5", 2): "AMP_OUT", ("2.5", 3): "MIC_GND"}


def k1_net(jack, number):
    return K1_NETS.get((jack, number), f"K1_{jack.replace('.', '')}_{number}")


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

# (Rev 10's 10-pin header, every jack pin out for breadboard jumpers, is gone
# in rev 11: the mic pins go straight to the circuit, K1_NETS)

# Each jack's size beside its body, on both silkscreens: the jacks go in from
# the back
for layer in ("F.SilkS", "B.SilkS"):
    for jack in ("3.5", "2.5"):
        bore_x = K1[jack]["bore"][0][0]
        # beside its bore; the 2.5's inside its own body (C1 is beside it)
        x_mm, y_mm = k1_at(bore_x, {"3.5": 15.0, "2.5": 11.0}[jack])
        board.add_text(x_mm, y_mm, jack, layer=layer, size=0.8, thickness=0.12)

# Each jack's body outline on the floor-side silkscreen, where the jacks
# sit, so nothing else lands under them: the measured bodies
# (k1_jacks.json) plus a margin. A marked-up photo (2026-11-06) put the
# back edges up to 0.8 mm further out, most of it parallax off the 10 mm
# tall bodies. Then squared with the real parts (user, 2026-11-06): the
# 3.5's grows to cover its own pads whole, and the 2.5's, which in real life overhangs
# neither the board edge nor the 3.5, slides onto the board and down to
# where the 3.5's ends
K1_BODY_MARGIN = 0.5
K1_SILK_CLEAR = 0.3  # silkscreen off pads' mask openings and the board edge
k1_outlines = {}
for jack, pad_style in (("3.5", K1_PIN_35), ("2.5", K1_PIN_25)):
    corners = [k1_at(x_mm, y_mm) for x_mm, y_mm in K1[jack]["body"]]
    corners += [k1_at(pin["x"], pin["y"]) for pin in K1[jack]["pins"]]
    pad_radius = pad_style["size"] / 2
    left = min(x_mm for x_mm, _ in corners) - K1_BODY_MARGIN
    top = min(y_mm for _, y_mm in corners) - K1_BODY_MARGIN
    right = max(x_mm for x_mm, _ in corners) + K1_BODY_MARGIN
    bottom = max(y_mm for _, y_mm in corners) + K1_BODY_MARGIN
    if jack == "3.5":
        # every pad whole inside: pads reach pad_radius past their centres
        pads = [k1_at(pin["x"], pin["y"]) for pin in K1[jack]["pins"]]
        reach = pad_radius + K1_SILK_CLEAR
        left = min(left, min(x_mm for x_mm, _ in pads) - reach)
        top = min(top, min(y_mm for _, y_mm in pads) - reach)
        right = max(right, max(x_mm for x_mm, _ in pads) + reach)
        bottom = max(bottom, max(y_mm for _, y_mm in pads) + reach)
    else:
        shift_x = max(0, BX + K1_SILK_CLEAR - left)
        shift_y = max(0, k1_outlines["3.5"][3] - top)
        left, right = left + shift_x, right + shift_x
        top, bottom = top + shift_y, bottom + shift_y
    k1_outlines[jack] = (left, top, right, bottom)
    board.add_rect(left, top, right - left, bottom - top, layer="B.SilkS", width=0.12)

# The mic's passives (rev 11), bench-proven 2026-11-05 on the Heltec ADC with
# the capsule, a BTECH lapel and a Disco32 + Peltor (memory: mic-board).
# Through-hole, resistors lying flat (RESISTOR_PITCH), beside the K1
# jacks. Each value on the silkscreen beside its holes. They go in from the
# BACK (KiCad's B: the floor side, the Heltec's and the screen's), pointing
# down into the case: the lid side has only a few mm. Anything up to
# ~11 mm tall fits under the hanging Heltec (11.07 mm to its PCB, measured
# 2026-11-06), so the resistors and C2 stand in a row under it, midway
# between its header rows; C1's can (11.93) does not, so it sits west of
# the Heltec, below the 3.5 jack
BACK_SILK = "B.SilkS"
C1_DIAMETER = 6.52
# A real resistor's body is ~7 mm: its legs bent down past it need 7.62 mm
# (0.3") between holes (user, 2026-11-07). Ceramic caps stand on 2.54
RESISTOR_PITCH = 7.62
PASSIVE = []  # (label, x, y, pin 1 net, pin 2 net, value, pitch)
# Ring bias: a radio-like ~3 V through ~3k, and FILTERED: the Disco32 is
# nearly silent on the MAX9814's own ~2 V / 2.2k bias, and an unfiltered
# resistor from 3V3 puts the USB frames' 25 Hz on the ring
# West end of the Heltec's outline: its antenna tip (x 132.24) plus the
# labels' half width and a margin, leaving the east end to AMP2
PASSIVE_ROW_Y = BY + 24.0
PASSIVE.append(("R1", BX + 21.0, PASSIVE_ROW_Y, "Ve", "BIAS", "1k", RESISTOR_PITCH))
PASSIVE.append(("R2", BX + 24.5, PASSIVE_ROW_Y, "BIAS", "MIC_IN", "2k", RESISTOR_PITCH))
# The bias filter's can: its - leg on the MIC ground (MIC_GND), never
# elsewhere: on any other ground point it put the 25 Hz back
PASSIVE.append(("C1", BX + 15.0, BY + 26.6, "BIAS", "MIC_GND", "100u", 2.5))
# Anti-alias at the ADC: the MAX9814 output carries ~70 mV at 20-500 kHz,
# which the ESP32's ADC folds into the voice band (1k + 100n: quiet floor
# 92 -> 14 mV). 104 can become 47n (3.4 kHz) without touching the board.
# Its ground is the MIC ground, as on the bench
PASSIVE.append(("R3", BX + 28.0, PASSIVE_ROW_Y, "MIC_OUT", "ADC", "1k", RESISTOR_PITCH))
PASSIVE.append(("C2", BX + 31.5, PASSIVE_ROW_Y, "ADC", "MIC_GND", "104", 2.54))
# RF bypass at the ring, as OpenVLM has it: NOT fitted unless the radio's own
# transmissions get into the mic (the open 160 ms chop, 2026-11-05)
# South of the 2.5 jack's body (it ends at y+32)
# (and north of the speaker amp's outline, which their leads must not reach)
PASSIVE.append(("C3", BX + 12.0, BY + 33.5, "MIC_IN", "MIC_GND", "1n", 2.54))
PASSIVE.append(("C4", BX + 15.5, BY + 33.5, "MIC_IN", "MIC_GND", "470p", 2.54))
board.add_text(BX + 18.6, BY + 36.4, "DNP", layer=BACK_SILK, size=0.8,
               thickness=0.12)  # after C4's value
# Labels beside a pin: name north of the north pin (frame (x, 0) = x mm
# north), value south of the south one, this far past the pin
LABEL_GAP = 1.63
for label, x_mm, y_mm, net_1, net_2, value, pitch in PASSIVE:
    if label != "C1":
        # North-south: pin 1 south
        label_dy = pitch / 2 + LABEL_GAP
        board.add_header(x_mm, y_mm, pins=2, label=label, angle=90, pitch=pitch,
                         pad_labels=["1", "2"], pad_nets=[net_1, net_2], ref_at=(label_dy, 0),
                         ref_layer=BACK_SILK)
        board.add_text(x_mm, y_mm + label_dy, value, layer=BACK_SILK, size=0.8, thickness=0.12)
    else:
        # The can: + (pin 1) west, standing on the floor side, its body
        # outlined (6.52 mm across, micrometer 2026-11-06)
        board.add_circle(x_mm, y_mm, C1_DIAMETER, layer=BACK_SILK, width=0.12)
        board.add_header(x_mm, y_mm, pins=2, label=label, pitch=pitch,
                         pad_labels=["+", "-"], pad_nets=[net_1, net_2], ref_layer=BACK_SILK)
        board.add_text(x_mm - 2.6, y_mm, "+", layer=BACK_SILK, size=0.8, thickness=0.12)
        board.add_text(x_mm, y_mm + 2.0, value, layer=BACK_SILK, size=0.8, thickness=0.12)

# The ONE place the mic's ground meets the board's: beside the amp board's
# GND pin, as on the bench (the amp's GND row, one wire to the Heltec). A
# wire link, so a star-ground mistake can be undone with a cutter
# Its name and value as one label, below it: above it is the Heltec's edge
# Fitted from the back with the passives, so labelled there
board.add_header(MIC_X + 1.27, BY + 38.4, pins=2, label="JP1",
                 pad_labels=["MIC_GND", "GND"], pad_nets=["MIC_GND", "GND"], ref_hidden=True)
board.add_text(MIC_X + 1.27, BY + 40.0, "JP1 0R", layer=BACK_SILK, size=0.8, thickness=0.12)

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
# And just under BAT's GND pad: rev 10.2's routes boxed the pad in (VBAT
# over its top to SW, SD_CLK right under the connector row), leaving one
# thermal spoke (DRC: starved thermal, 2026-11-08). The via pushes SD_CLK
# further south and gives the pad a spoke down to it
board.add_via(BX + 44.7, BY + 5.9, "GND")

board.save("open-oswst.kicad_pcb")
