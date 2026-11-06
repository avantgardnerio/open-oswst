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
REV = "10.1"

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
board.add_jst_ph(cx, cy, pins=2, label="SPK",
                 pad_labels=["-", "+"], pad_nets=["SPK-", "SPK+"]); cx += JST2_STEP
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
                 ref_at=(-12, 2.5))  # below the row, west: AMP2's outline is above it

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

# Room for a second amp board later, on the lid side against the east edge,
# centred between the Heltec's header rows (not fitted in rev 10.1, only
# outlined): 31.41 x 16.54 (user's measurement, 2026-11-06). The passive
# row under the Heltec keeps well west of it: their joints are on this side
AMP2_LENGTH = 31.41
AMP2_WIDTH = 16.54
amp2_right = BX + BW - 0.3
amp2_centre_y = HY + HELTEC_WIDTH / 2
board.add_rect(amp2_right - AMP2_LENGTH, amp2_centre_y - AMP2_WIDTH / 2, AMP2_LENGTH,
               AMP2_WIDTH, layer="F.SilkS", width=0.12)
board.add_text(amp2_right - AMP2_LENGTH / 2, amp2_centre_y, "AMP2", size=1, thickness=0.15)

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
CAPSULE_FROM_HEADER = 25.0  # 1.5 mm past the amp board's far edge, clear of MIC_IN's joints
# Far enough east that the capsule, past the amp board's far (west) edge,
# clears the speaker amp's outline (AMP_* below, x 132.2): its holes by 0.15 mm
MIC_X = BX + 45.2
MIC_Y = BY + 47
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
# 9.74 mm across (micrometer, 2026-11-06), on the floor side: 1 mm north of
# the amp boards' row keeps its edge ~1 mm off the speaker amp's Vin pin,
# which is soldered on this side
CAPSULE_X = MIC_X - CAPSULE_FROM_HEADER
CAPSULE_Y = MIC_Y - 1.0
CAPSULE_DIAMETER = 9.74
board.add_circle(CAPSULE_X, CAPSULE_Y, CAPSULE_DIAMETER, layer="B.SilkS", width=0.12)
board.add_header(CAPSULE_X, CAPSULE_Y, pins=2, label="CAPSULE", angle=270,
                 pad_labels=["+", "-"], pad_nets=["CAPSULE", "MIC_GND"], ref_at=(-8.0, 0),
                 ref_layer="B.SilkS")  # its name on its own side, clear of both outlines
# Which hole is which, on both sides (the capsule faces the floor, the amp
# board's leads come from the lid side): "+" (pin 1) north
for layer in ("F.SilkS", "B.SilkS"):
    # MIC_IN's marks east of it, "+" south; the capsule's west (east is the
    # amp board's edge), "+" north
    for x_mm, y_mm, plus_south in ((MIC_X - MIC_Q1 + 1.8, MIC_Y, True),
                                   (CAPSULE_X - 1.8, CAPSULE_Y, False)):
        plus_dy = 1.27 if plus_south else -1.27
        board.add_text(x_mm, y_mm + plus_dy, "+", layer=layer, size=0.8, thickness=0.12)
        board.add_text(x_mm, y_mm - plus_dy, "-", layer=layer, size=0.8, thickness=0.12)
# The amp board's outline, so nothing else lands under it
board.add_rect(MIC_X - MIC_FAR_EDGE, MIC_Y - MIC_WIDTH / 2, MIC_FAR_EDGE + MIC_NEAR_EDGE,
               MIC_WIDTH, layer="F.SilkS", width=0.12)
AMP_X = BX + 3 + 3 * 2.54
# Down by the bottom-left mounting hole (rev 10, clearing the top left for
# the Kenwood jacks): the header row stays 4.5 mm above the hole, room for
# the M2 screw head (3.8 mm), the breakout's edge past its pins, and 1 mm
AMP_Y = BY + BH - INSET_TB - 4.5
board.add_header(AMP_X, AMP_Y, pins=7, label="AMP", angle=180,
                 pad_labels=["Vin", "GND", "SD", "GAIN", "DIN", "BCLK", "LRC"],
                 pad_nets=["Ve", "GND", None, None, "DIN", "BCLK", "LRC"],
                 ref_at=(0, -4.6))  # turned 180: south, below the breakout's outline
board.add_header(AMP_X, AMP_Y - 12.954, pins=2, label="AMP_SPK", angle=180,
                 pitch=3.5, pad_labels=["+", "-"], pad_nets=["SPK+", "SPK-"])
# The breakout's outline, so nothing else lands under it (measured off a
# marked-up photo 2026-11-06): 0.93 mm past the end pins, 16.1 mm north of
# the pin row (the speaker terminal end), ~2.2 mm south of it (not marked)
AMP_PIN_SPAN = 6 * 2.54
AMP_SIDE_EDGE = 0.93
AMP_FAR_EDGE = 16.1
AMP_NEAR_EDGE = 2.2
board.add_rect(AMP_X - AMP_PIN_SPAN / 2 - AMP_SIDE_EDGE, AMP_Y - AMP_FAR_EDGE,
               AMP_PIN_SPAN + 2 * AMP_SIDE_EDGE, AMP_FAR_EDGE + AMP_NEAR_EDGE,
               layer="F.SilkS", width=0.12)


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
# capsule (open with a plug in), 2.5 sleeve = the headset mic's ground. The
# rest (the speaker half) go nowhere yet
K1_NETS = {("3.5", 3): "PTT", ("3.5", 4): "CAPSULE", ("3.5", 5): "MIC_IN",
           ("2.5", 3): "MIC_GND"}


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
# Through-hole, legs 2.54 mm apart (resistors stand up), beside the K1
# jacks. Each value on the silkscreen beside its holes. They go in from the
# BACK (KiCad's B: the floor side, the Heltec's and the screen's), pointing
# down into the case: the lid side has only a few mm. Anything up to
# ~11 mm tall fits under the hanging Heltec (11.07 mm to its PCB, measured
# 2026-11-06), so the resistors and C2 stand in a row under it, midway
# between its header rows; C1's can (11.93) does not, so it sits west of
# the Heltec, below the 3.5 jack
BACK_SILK = "B.SilkS"
C1_DIAMETER = 6.52
PASSIVE = []  # (label, x, y, pin 1 net, pin 2 net, value, pitch)
# Ring bias: a radio-like ~3 V through ~3k, and FILTERED: the Disco32 is
# nearly silent on the MAX9814's own ~2 V / 2.2k bias, and an unfiltered
# resistor from 3V3 puts the USB frames' 25 Hz on the ring
# West end of the Heltec's outline: its antenna tip (x 132.24) plus the
# labels' half width and a margin, leaving the east end to AMP2
PASSIVE_ROW_Y = BY + 24.0
PASSIVE.append(("R1", BX + 21.0, PASSIVE_ROW_Y, "Ve", "BIAS", "1k", 2.54))
PASSIVE.append(("R2", BX + 24.5, PASSIVE_ROW_Y, "BIAS", "MIC_IN", "2k", 2.54))
# The bias filter's can: its - leg on the MIC ground (MIC_GND), never
# elsewhere: on any other ground point it put the 25 Hz back
PASSIVE.append(("C1", BX + 15.0, BY + 26.6, "BIAS", "MIC_GND", "100u", 2.5))
# Anti-alias at the ADC: the MAX9814 output carries ~70 mV at 20-500 kHz,
# which the ESP32's ADC folds into the voice band (1k + 100n: quiet floor
# 92 -> 14 mV). 104 can become 47n (3.4 kHz) without touching the board.
# Its ground is the MIC ground, as on the bench
PASSIVE.append(("R3", BX + 28.0, PASSIVE_ROW_Y, "MIC_OUT", "ADC", "1k", 2.54))
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
# north), value south of the south one
LABEL_GAP = 2.9
for label, x_mm, y_mm, net_1, net_2, value, pitch in PASSIVE:
    if pitch == 2.54:
        # Standing up, north-south: pin 1 south
        board.add_header(x_mm, y_mm, pins=2, label=label, angle=90,
                         pad_labels=["1", "2"], pad_nets=[net_1, net_2], ref_at=(LABEL_GAP, 0),
                         ref_layer=BACK_SILK)
        board.add_text(x_mm, y_mm + LABEL_GAP, value, layer=BACK_SILK, size=0.8, thickness=0.12)
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

board.save("open-oswst.kicad_pcb")
