"""Draw the K1 jack pin map (k1_jack_pins.json) as a schematic card, 1280x720 PNG.

  plot_pins.py <output.png>

Each contact (tip, ring, sleeve) is a line from its pin number; a switch pin is an amber
blade resting on its contact, the way a normally-closed jack switch sits until a plug lifts
it. Used as the end card of the bench video (scripts/bench_video.py --end-card).
"""
import json
import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

PINS = json.loads((pathlib.Path(__file__).parent / "k1_jack_pins.json").read_text())
JACK_USE = {"3.5": "K1 mic + PTT", "2.5": "K1 speaker"}
CONTACT_ORDER = ["tip", "ring", "sleeve"]

WIDTH, HEIGHT = 1280, 720
FONT_DIR = "/usr/share/fonts/truetype/dejavu/"
TITLE = ImageFont.truetype(FONT_DIR + "DejaVuSansCondensed-Bold.ttf", 44)
SMALL = ImageFont.truetype(FONT_DIR + "DejaVuSansCondensed-Bold.ttf", 24)
MONO = ImageFont.truetype(FONT_DIR + "DejaVuSansMono.ttf", 22)
MONO_BOLD = ImageFont.truetype(FONT_DIR + "DejaVuSansMono-Bold.ttf", 22)
INK = (232, 236, 240)
MUTED = (150, 162, 172)
AMBER = (240, 164, 58)
GREEN = (95, 211, 148)
BACKGROUND = (30, 35, 41)
RULE = (62, 71, 80)


def draw_jack(draw, left, top, jack):
    pins = PINS[jack]
    draw.text((left, top), f"{jack} mm jack", font=TITLE, fill=INK)
    draw.text((left, top + 54), JACK_USE[jack], font=SMALL, fill=MUTED)
    y = top + 120
    for contact in CONTACT_ORDER:
        pin = next(number for number, info in pins.items() if info["contact"] == contact)
        switch_pin = next((number for number, info in pins.items()
                           if info.get("closed_to_without_plug") == int(pin)), None)
        draw.text((left, y - 13), f"pin {pin}", font=MONO_BOLD, fill=GREEN)
        draw.line((left + 90, y, left + 230, y), fill=GREEN, width=4)
        draw.ellipse((left + 226, y - 6, left + 238, y + 6), fill=GREEN)
        draw.text((left + 250, y - 13), contact.upper(), font=MONO_BOLD, fill=INK)
        draw.text((left + 250, y + 14), pins[pin]["k1"], font=MONO, fill=MUTED)
        if switch_pin is None:
            y += 80
            continue
        # Normally-closed switch: a blade from the switch pin resting on the contact line.
        switch_y = y + 52
        draw.text((left, switch_y - 13), f"pin {switch_pin}", font=MONO_BOLD, fill=AMBER)
        draw.line((left + 90, switch_y, left + 130, switch_y), fill=AMBER, width=4)
        draw.line((left + 130, switch_y, left + 185, y + 4), fill=AMBER, width=4)
        draw.text((left + 250, switch_y - 4), "switch: opens on plug-in", font=MONO, fill=AMBER)
        y += 116


def main():
    image = Image.new("RGB", (WIDTH, HEIGHT), BACKGROUND)
    draw = ImageDraw.Draw(image)
    draw_jack(draw, 70, 50, "3.5")
    draw_jack(draw, 690, 50, "2.5")
    draw.line((70, 600, WIDTH - 70, 600), fill=RULE, width=2)
    draw.text((70, 620), "Mapped in two sweeps, plugs out and plugs in, about 20 s each.", font=SMALL, fill=INK)
    draw.text((70, 656), "Jumperless V5 + overhead camera + Claude. No datasheet, no meter.", font=SMALL, fill=MUTED)
    image.save(sys.argv[1])
    print("wrote", sys.argv[1])


if __name__ == "__main__":
    main()
