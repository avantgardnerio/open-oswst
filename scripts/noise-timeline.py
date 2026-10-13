#!/usr/bin/env python3
"""Noise over time on a walk: the handheld's and the echo station's.

Two ways the logs tell us the noise a radio hears:
  - "RX state: noise avg=..": the radio's own reading every 10 s, taken
    while nobody is transmitting (the radio samples the channel's RSSI).
  - Inside each received packet: the radio reports the packet's RSSI
    (signal + noise) and its SNR (signal / noise), so RSSI - SNR is the
    noise underneath that packet. Only for packets with SNR below
    SNR_TRUSTED: the SX1262's SNR tops out at ~+13, and a packet that
    strong hides the noise.
If the two disagree, the reading is not measuring what reaches the
demodulator.

Panels, one time axis (UTC, lined up from the GPS lines):
  1. how far the handheld was from the echo (and from the repeater, if given),
     with a tick for each push-to-talk
  2. the handheld's noise
  3. the echo station's noise

Usage:
    .venv/bin/python scripts/noise-timeline.py HANDHELD_LOG ECHO_LOG [REPEATER_LAT,LON]
Writes noise-NNNN.png next to the handheld log (NNNN = its number).
"""

import datetime as dt
import importlib.util
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.dates as mdates
import matplotlib.pyplot as plt

# range-report.py has the log parsing and GPS clock alignment; its name has a
# dash, so it's loaded by path
spec = importlib.util.spec_from_file_location("range_report", Path(__file__).parent / "range-report.py")
range_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(range_report)

# Packets at or above this SNR say nothing about the noise (see above)
SNR_TRUSTED = 10

NOISE_READING = re.compile(r"RX state: noise avg=(-?\d+) max=(-?\d+)")
PACKET = re.compile(r"^RX (?:end|CRC error) \[\d+B\].* rssi=(-?\d+) snr=(-?\d+)")

READING_COLOUR, PACKET_COLOUR = "#2a78d6", "#eb6834"
ECHO_COLOUR, REPEATER_COLOUR = "#2a78d6", "#4a3aa7"
INK, MUTED, GRID, SURFACE = "#0b0b0b", "#52514e", "#e9e8e4", "#fcfcfb"


def main():
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    hand_path, echo_path = Path(sys.argv[1]), Path(sys.argv[2])
    repeater = tuple(map(float, sys.argv[3].split(","))) if len(sys.argv) == 4 else None

    hand, echo = range_report.parse(hand_path), range_report.parse(echo_path)
    hand_gps, echo_gps = range_report.gps_points(hand), range_report.gps_points(echo)
    station = range_report.median_position(echo_gps)

    out = hand_path.parent / f"noise-{hand_path.stem}.png"
    plot(out, hand, hand_gps, echo, echo_gps, station, repeater)
    print(out)


def utc(lines_gps, t):
    """A log timestamp (ms since boot) on the UTC clock."""
    offset = range_report.clock_offset(lines_gps)
    return dt.datetime.fromtimestamp((t + offset) / 1000, dt.timezone.utc)


def noise_series(lines, gps):
    """(times, readings) of the 10 s noise reading, and (times, noise) under weak packets."""
    readings, packets = ([], []), ([], [])
    for t, _, msg in lines:
        m = NOISE_READING.search(msg)
        if m:
            readings[0].append(utc(gps, t))
            readings[1].append(int(m.group(1)))
            continue
        m = PACKET.search(msg)
        if m and int(m.group(2)) < SNR_TRUSTED:
            packets[0].append(utc(gps, t))
            packets[1].append(int(m.group(1)) - int(m.group(2)))
    return readings, packets


def plot(out, hand, hand_gps, echo, echo_gps, station, repeater):
    plt.rcParams.update({"font.family": "sans-serif", "font.size": 9, "axes.edgecolor": "#c3c2b7",
                         "axes.labelcolor": MUTED, "xtick.color": MUTED, "ytick.color": MUTED})
    fig, (ax_dist, ax_hand, ax_echo) = plt.subplots(
        3, 1, figsize=(15, 9), sharex=True, gridspec_kw={"height_ratios": [1, 1.4, 1.4]})

    # 1. Where the handheld was
    fixes = [p for p in hand_gps if p["pos"]]
    times = [p["utc"] for p in fixes]
    ax_dist.plot(times, [range_report.distance_m(p["pos"], station) for p in fixes],
                 color=ECHO_COLOUR, linewidth=2, label="from the echo")
    if repeater:
        ax_dist.plot(times, [range_report.distance_m(p["pos"], repeater) for p in fixes],
                     color=REPEATER_COLOUR, linewidth=2, linestyle=(0, (4, 2)), label="from the repeater")
    for t, _, msg in hand:
        if "PTT pressed" in msg:
            ax_dist.axvline(utc(hand_gps, t), color=MUTED, linewidth=0.6, alpha=0.5, ymin=0.92)
    ax_dist.set_ylabel("handheld distance, m")
    ax_dist.set_title("Ticks along the top: push-to-talk", loc="right", fontsize=8, color=MUTED)
    ax_dist.legend(loc="upper left", frameon=False, fontsize=8.5)

    # 2 and 3. The noise each radio heard
    for ax, lines, gps, name in ((ax_hand, hand, hand_gps, "Handheld"), (ax_echo, echo, echo_gps, "Echo station")):
        readings, packets = noise_series(lines, gps)
        ax.plot(readings[0], readings[1], color=READING_COLOUR, linewidth=1, marker="o", markersize=2.5,
                label="radio's 10 s noise reading (avg)")
        ax.scatter(packets[0], packets[1], s=14, color=PACKET_COLOUR, edgecolors=SURFACE, linewidths=0.5,
                   zorder=3, label=f"noise under a received packet (RSSI - SNR, SNR < {SNR_TRUSTED})")
        ax.set_ylabel("dBm")
        ax.set_title(name, loc="left", fontsize=10, color=INK)
        ax.legend(loc="upper left", frameon=False, fontsize=8.5, ncol=2)

    # Same dBm scale on both radios, so they compare at a glance
    low = min(ax_hand.get_ylim()[0], ax_echo.get_ylim()[0])
    high = max(ax_hand.get_ylim()[1], ax_echo.get_ylim()[1]) + 6  # room for the legend
    for ax in (ax_hand, ax_echo):
        ax.set_ylim(low, high)
    for ax in (ax_dist, ax_hand, ax_echo):
        ax.grid(color=GRID, linewidth=0.8)
        ax.set_axisbelow(True)
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
    ax_echo.xaxis.set_major_formatter(mdates.DateFormatter("%H:%M", tz=dt.timezone.utc))
    ax_echo.set_xlabel("UTC")
    fig.suptitle("Noise over the walk", x=0.06, ha="left", fontsize=12, color=INK)
    fig.tight_layout()
    fig.savefig(out, dpi=130, facecolor=SURFACE)


if __name__ == "__main__":
    main()
