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
                           "SD_CMD", "SD_D0", "Ve", "Ve", None, "GND"])

# MAX9814 mic amp (Amazon clone of Adafruit 1713), turned 90 degrees (rev
# 11): its header runs north-south and the board lies flat over this one,
# reaching east. Its own capsule is gone: the clone's two capsule pads come
# down on short flying leads to MIC_IN below, and the mic is the K1 jack's
# ring (or the board's capsule through the ring's NC switch). Clone layout
# (measured off a marked-up photo 2026-11-06, the capsule pads checked
# with the real board on a printed coupon, models/mic_coupon.py): from its
# header, the capsule pads (Q1) 18.7 mm on, centred on GAIN, 2.54 apart;
# its far edge 23.5 mm on, its near edge 1.6 mm behind; 13.8 mm wide
MIC_X = BX + 34   # the outline's west edge clear of the SPK connector
MIC_Y = BY + 47
MIC_Q1 = 18.7
MIC_FAR_EDGE = 23.5
MIC_NEAR_EDGE = 1.6
MIC_WIDTH = 13.8
# GAIN to VDD = 40 dB: 60 dB (floating) swung rail to rail on the bench, 50
# clipped the Disco32. GND is the mic ground (JP1 joins it to the board's)
board.add_header(MIC_X, MIC_Y, pins=5, label="MIC", angle=90,
                 pad_labels=["AR", "OUT", "GAIN", "VDD", "GND"],
                 pad_nets=[None, "MIC_OUT", "Ve", "Ve", "MIC_GND"])
# Flying-lead holes, right under the clone's old capsule pads. "+" is the
# amp's input (MICBIAS through its 2.2k, into MICIN through 0.1 uF); "-" is
# the amp board's ground, which on the bench had to be the ground of every
# mic part (the bias filter cap's above all: anywhere else put the Heltec
# USB frames' 25 Hz into the ring, 2026-11-05)
# Names turned with the header (frame (0, -y) = y mm west, (x, 0) = x mm
# north), clear of the pads and the amp board's outline
board.add_header(MIC_X + MIC_Q1, MIC_Y, pins=2, label="MIC_IN", angle=90,
                 pad_labels=["+", "-"], pad_nets=["MIC_IN", "MIC_GND"], ref_at=(0, -4.5))
# The board's own mic: the 9x7 mm electret capsule, on the K1 3.5 jack's
# ring NC switch (net CAPSULE, K1_NETS): plugging a headset in opens the
# switch and the headset's mic, on the ring, takes over. Between MIC_IN and
# VOL, east of the amp board's outline
CAPSULE_X = BX + 58.5
board.add_header(CAPSULE_X, MIC_Y, pins=2, label="CAPSULE", angle=90,
                 pad_labels=["+", "-"], pad_nets=["CAPSULE", "MIC_GND"], ref_at=(3.5, 0))
# Which hole is which, on both sides (the amp board sits on the back): the
# 2-pin headers turned 90 degrees put "+" (pin 1) south
for layer in ("F.SilkS", "B.SilkS"):
    for x_mm in (MIC_X + MIC_Q1, CAPSULE_X):
        board.add_text(x_mm + 1.8, MIC_Y + 1.27, "+", layer=layer, size=0.8, thickness=0.12)
        board.add_text(x_mm + 1.8, MIC_Y - 1.27, "-", layer=layer, size=0.8, thickness=0.12)
# The amp board's outline, so nothing else lands under it
board.add_rect(MIC_X - MIC_NEAR_EDGE, MIC_Y - MIC_WIDTH / 2, MIC_NEAR_EDGE + MIC_FAR_EDGE,
               MIC_WIDTH, layer="F.SilkS", width=0.12)
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

# SPK connector (rotated 90°), just right of the amp breakout, beside its
# AMP_SPK terminals (rev 10: the left edge went to the Kenwood jacks)
board.add_jst_ph(BX + 24, BY + 46, pins=2, label="SPK", angle=90,
                 pad_labels=["-", "+"], pad_nets=["SPK-", "SPK+"])

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
        x_mm, y_mm = k1_at(bore_x, 15.0)
        board.add_text(x_mm, y_mm, jack, layer=layer, size=0.8, thickness=0.12)

# Each jack's body outline on the back (floor side), where the jacks sit, so
# nothing else lands under them: the measured bodies (k1_jacks.json) plus a
# margin. On the fab layer, not the silkscreen: the bodies sit over their
# own pins and past the board edge, so a printed outline would cross both. A marked-up photo (2026-11-06) put the back edges up to 0.8 mm
# further out, most of it parallax off the 10 mm tall bodies
# Then squared with the real parts (user, 2026-11-06): the 3.5's grows to
# cover its own pads whole, and the 2.5's, which in real life overhangs
# neither the board edge nor the 3.5, slides onto the board and down to
# where the 3.5's ends
K1_BODY_MARGIN = 0.5
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
        left = min(left, min(x_mm for x_mm, _ in pads) - pad_radius)
        top = min(top, min(y_mm for _, y_mm in pads) - pad_radius)
        right = max(right, max(x_mm for x_mm, _ in pads) + pad_radius)
        bottom = max(bottom, max(y_mm for _, y_mm in pads) + pad_radius)
    else:
        shift_x = max(0, BX - left)
        shift_y = max(0, k1_outlines["3.5"][3] - top)
        left, right = left + shift_x, right + shift_x
        top, bottom = top + shift_y, bottom + shift_y
    k1_outlines[jack] = (left, top, right, bottom)
    board.add_rect(left, top, right - left, bottom - top, layer="B.Fab", width=0.12)

# The mic's passives (rev 11), bench-proven 2026-11-05 on the Heltec ADC with
# the capsule, a BTECH lapel and a Disco32 + Peltor (memory: mic-board).
# Through-hole, legs 2.54 mm apart (resistors stand up), in the strip
# between the K1 jacks' bodies (on the back, to x+15) and the Heltec's
# headers. Each value on the silkscreen beside its holes. They go in from
# the BACK, pointing down into the case: the front has only a few mm under
# the lid (rev 10.1), so their labels are on the back silkscreen
BACK_SILK = "B.SilkS"
PASSIVE = []  # (label, x, y, pin 1 net, pin 2 net, value, pitch)
# Ring bias: a radio-like ~3 V through ~3k, and FILTERED: the Disco32 is
# nearly silent on the MAX9814's own ~2 V / 2.2k bias, and an unfiltered
# resistor from 3V3 puts the USB frames' 25 Hz on the ring
PASSIVE.append(("R1", BX + 17.5, BY + 16.8, "Ve", "BIAS", "1k", 2.54))
PASSIVE.append(("R2", BX + 21.0, BY + 16.8, "BIAS", "MIC_IN", "2k", 2.54))
# The bias filter's can: its - leg on the MIC ground (MIC_GND), never
# elsewhere: on any other ground point it put the 25 Hz back
PASSIVE.append(("C1", BX + 19.25, BY + 22.5, "BIAS", "MIC_GND", "100u", 2.5))
# Anti-alias at the ADC: the MAX9814 output carries ~70 mV at 20-500 kHz,
# which the ESP32's ADC folds into the voice band (1k + 100n: quiet floor
# 92 -> 14 mV). 104 can become 47n (3.4 kHz) without touching the board.
# Its ground is the MIC ground, as on the bench
PASSIVE.append(("R3", BX + 17.5, BY + 28.8, "MIC_OUT", "ADC", "1k", 2.54))
PASSIVE.append(("C2", BX + 21.0, BY + 28.8, "ADC", "MIC_GND", "104", 2.54))
# RF bypass at the ring, as OpenVLM has it: NOT fitted unless the radio's own
# transmissions get into the mic (the open 160 ms chop, 2026-11-05)
# South of the 2.5 jack's body (it ends at y+32)
PASSIVE.append(("C3", BX + 12.0, BY + 34.5, "MIC_IN", "MIC_GND", "1n", 2.54))
PASSIVE.append(("C4", BX + 15.5, BY + 34.5, "MIC_IN", "MIC_GND", "470p", 2.54))
board.add_text(BX + 18.6, BY + 37.4, "DNP", layer=BACK_SILK, size=0.8,
               thickness=0.12)  # after C4's value
for label, x_mm, y_mm, net_1, net_2, value, pitch in PASSIVE:
    if pitch == 2.54:
        # Standing up, north-south: pin 1 south. Its name above the north
        # pin (frame (x, 0) = x mm north), its value below the south one
        board.add_header(x_mm, y_mm, pins=2, label=label, angle=90,
                         pad_labels=["1", "2"], pad_nets=[net_1, net_2], ref_at=(2.9, 0),
                         ref_layer=BACK_SILK)
        board.add_text(x_mm, y_mm + 2.9, value, layer=BACK_SILK, size=0.8, thickness=0.12)
    else:
        # The can: + (pin 1) west
        board.add_header(x_mm, y_mm, pins=2, label=label, pitch=pitch,
                         pad_labels=["+", "-"], pad_nets=[net_1, net_2], ref_layer=BACK_SILK)
        board.add_text(x_mm - 2.6, y_mm, "+", layer=BACK_SILK, size=0.8, thickness=0.12)
        board.add_text(x_mm, y_mm + 2.0, value, layer=BACK_SILK, size=0.8, thickness=0.12)

# The ONE place the mic's ground meets the board's: beside the amp board's
# GND pin, as on the bench (the amp's GND row, one wire to the Heltec). A
# wire link, so a star-ground mistake can be undone with a cutter
# Its name and value as one label: above it are J2's pins, below the amp board
# Fitted from the back with the passives, so labelled there
board.add_header(MIC_X + 1.27, BY + 38.4, pins=2, label="JP1",
                 pad_labels=["MIC_GND", "GND"], pad_nets=["MIC_GND", "GND"], ref_hidden=True)
board.add_text(MIC_X + 1.27, BY + 37.0, "JP1 0R", layer=BACK_SILK, size=0.8, thickness=0.12)

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
