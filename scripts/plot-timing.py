#!/usr/bin/env python3
"""Parse monitor.log and plot TX/RX timing diagram for each radio."""

import os
import re
import sys
from datetime import datetime

import matplotlib.pyplot as plt
import matplotlib.patches as mpatches

LOG_RE = re.compile(
    r"\[(\d{2}:\d{2}:\d{2}\.\d{3}) (ttyACM\d+)\] (.+)"
)

def parse_log(path):
    """Return list of (timestamp_ms, device, event) tuples."""
    events = []
    t0 = None
    for line in open(path):
        m = LOG_RE.match(line)
        if not m:
            continue
        ts_str, dev, msg = m.groups()
        ts = datetime.strptime(ts_str, "%H:%M:%S.%f")
        if t0 is None:
            t0 = ts
        ms = (ts - t0).total_seconds() * 1000

        if "TX start" in msg:
            events.append((ms, dev, "tx_start"))
        elif "TX end" in msg:
            events.append((ms, dev, "tx_end"))
        elif "TX waiting" in msg:
            events.append((ms, dev, "tx_wait"))
        elif "RX preamble" in msg:
            events.append((ms, dev, "rx_start"))
        elif "RX end" in msg:
            events.append((ms, dev, "rx_end"))
        elif "RX CRC" in msg or "RX error" in msg:
            events.append((ms, dev, "rx_end"))
    return events


def build_waves(events):
    """Build square wave segments per device per channel (tx/rx)."""
    # {dev: {"tx": [(start, end), ...], "rx": [(start, end), ...]}}
    waves = {}
    state = {}  # (dev, channel) -> start_ms

    for ms, dev, evt in events:
        if dev not in waves:
            waves[dev] = {"tx": [], "rx": []}

        if evt == "tx_start":
            state[(dev, "tx")] = ms
        elif evt == "tx_end":
            start = state.pop((dev, "tx"), None)
            if start is not None:
                waves[dev]["tx"].append((start, ms))
        elif evt == "tx_wait":
            # TX waiting means it hasn't started yet, just mark a brief blip
            pass
        elif evt == "rx_start":
            if (dev, "rx") not in state:
                state[(dev, "rx")] = ms
        elif evt == "rx_end":
            start = state.pop((dev, "rx"), None)
            if start is not None:
                waves[dev]["rx"].append((start, ms))

    return waves


def make_square_wave(segments):
    """Convert [(start, end), ...] into x,y arrays for a square wave line."""
    xs, ys = [], []
    for start, end in segments:
        xs.extend([start, start, end, end])
        ys.extend([0, 1, 1, 0])
    return xs, ys


def plot(waves):
    devices = sorted(waves.keys())
    n = len(devices)

    # Per-device colors, TX solid / RX dashed
    dev_colors = ["#e74c3c", "#2980b9", "#8e44ad", "#e67e22", "#2ecc71"]

    fig, ax = plt.subplots(figsize=(16, 4))

    handles = []
    for i, dev in enumerate(devices):
        color = dev_colors[i % len(dev_colors)]
        offset = i * 2.5  # vertical spacing between devices

        # TX
        tx_x, tx_y = make_square_wave(waves[dev]["tx"])
        tx_y = [y + offset + 1 for y in tx_y]
        ax.plot(tx_x, tx_y, color=color, linewidth=1.5, solid_capstyle="butt")
        ax.hlines(offset + 1, tx_x[0] if tx_x else 0, tx_x[-1] if tx_x else 0,
                  color=color, linewidth=0.5, alpha=0.3)

        # RX
        rx_x, rx_y = make_square_wave(waves[dev]["rx"])
        rx_y = [y + offset for y in rx_y]
        ax.plot(rx_x, rx_y, color=color, linewidth=1.5, linestyle="--",
                solid_capstyle="butt")
        ax.hlines(offset, rx_x[0] if rx_x else 0, rx_x[-1] if rx_x else 0,
                  color=color, linewidth=0.5, alpha=0.3)

        # Labels
        all_x = tx_x + rx_x
        x_min = min(all_x) if all_x else 0
        ax.text(x_min - 15, offset + 1.5, f"{dev} TX", ha="right", va="center",
                fontsize=9, fontweight="bold", color=color)
        ax.text(x_min - 15, offset + 0.5, f"{dev} RX", ha="right", va="center",
                fontsize=9, color=color)

        handles.append(mpatches.Patch(color=color, label=dev))

    # X ticks every 100ms
    all_times = []
    for dev in devices:
        for s, e in waves[dev]["tx"] + waves[dev]["rx"]:
            all_times.extend([s, e])
    if all_times:
        t_min = int(min(all_times) // 100) * 100
        t_max = int(max(all_times) // 100 + 1) * 100 + 100
        ax.set_xticks(range(t_min, t_max, 100))
        ax.set_xlim(t_min - 50, t_max)

    ax.set_xlabel("Time (ms)")
    ax.set_yticks([])
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    ax.spines["left"].set_visible(False)
    ax.set_title("Radio TX/RX Timing", fontweight="bold", fontsize=14)
    ax.legend(handles=handles, loc="upper right")

    plt.tight_layout()
    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "out", "timing.png")
    plt.savefig(out_path, dpi=150)
    print(f"Saved {out_path}")
    plt.show()


if __name__ == "__main__":
    script_dir = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.join(script_dir, "..", "out")
    path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(out_dir, "monitor.log")
    events = parse_log(path)
    if not events:
        print(f"No TX/RX events found in {path}", file=sys.stderr)
        sys.exit(1)
    waves = build_waves(events)
    plot(waves)
