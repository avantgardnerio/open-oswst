#!/usr/bin/env python3
"""What the handheld's receiver did during each echo replay that came via a repeater.

One panel per replay, on the conveyor's 80 ms bins (core/src/air.rs): the
echo sends packet n in bin 2n, the repeater relays it in bin 2n + 1. Rows:
  relayed copy  the repeater's copy of each packet: filled = received,
                dashed = nothing decoded, red outline = CRC error
  radio IRQs    every interrupt the radio raised: preamble, header,
                header error, packet end (black) or CRC error (red)
  direct copy   the echo's own copy, same marks
Under each box: RSSI / SNR of what was decoded there. Yellow: a preamble
followed by no header and no error until the next preamble, where that
span swallowed a relayed copy that never got decoded.

Timing: packet ends carry the radio's own microsecond stamp (at=..us).
Preamble / header lines only have the log's millisecond clock, which ticks
in 10 ms steps and runs on a different base: it's lined up on the packet
ends, so those marks are good to about +-5 ms.

Usage:
    .venv/bin/python scripts/rx-timeline.py HANDHELD_LOG ECHO_LOG [REPEATER_LAT,LON]
Writes timeline-NNNN-return.png next to the handheld log (NNNN = its number).
"""

import datetime as dt
import importlib.util
import re
import statistics
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.patches import Rectangle

# range-report.py has the log parsing and GPS clock alignment; its name has a
# dash, so it's loaded by path
spec = importlib.util.spec_from_file_location("range_report", Path(__file__).parent / "range-report.py")
range_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(range_report)

# Conveyor timing, ms (core/src/air.rs): a packet's air time, and the bins
AIR_MS, SLOT_MS, BIN_MS = 65.8, 160.0, 80.0
# A decoded packet belongs to a copy's box if it ended this close to the box's end
SAME_COPY_MS = 10

PACKET_LINE = re.compile(r"txid=(\d+) packet=(\d+) hops=(\d) rssi=(-?\d+) snr=(-?\d+)")

BLUE, ORANGE, RED, YELLOW = "#2a78d6", "#eb6834", "#d03b3b", "#fab219"
INK, MUTED, GRID, SURFACE = "#0b0b0b", "#52514e", "#e9e8e4", "#fcfcfb"


def main():
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    hand_path, echo_path = Path(sys.argv[1]), Path(sys.argv[2])
    repeater = tuple(map(float, sys.argv[3].split(","))) if len(sys.argv) == 4 else None

    hand, echo = range_report.parse(hand_path), range_report.parse(echo_path)
    hand_gps, echo_gps = range_report.gps_points(hand), range_report.gps_points(echo)
    station = range_report.median_position(echo_gps)
    replays = replay_sizes(echo, echo_gps)

    panels = []
    for txid, start, end in relayed_windows(hand):
        panel = read_window(hand, txid, start, end)
        if not panel:
            continue
        hand_offset = range_report.clock_offset(hand_gps)
        panel["utc"] = dt.datetime.fromtimestamp((start + hand_offset) / 1000, dt.timezone.utc)
        panel["packets"] = replay_size(replays, start + hand_offset, end + hand_offset) or panel["highest"] + 1
        pos = range_report.position_at(hand_gps, start)
        panel["from_echo"] = range_report.distance_m(pos, station)
        panel["from_repeater"] = range_report.distance_m(pos, repeater) if repeater else None
        panels.append(panel)

    out = hand_path.parent / f"timeline-{hand_path.stem}-return.png"
    plot(out, panels)
    print(out, f"({len(panels)} replays)")


# --- reading the logs -------------------------------------------------------


def replay_sizes(echo, echo_gps):
    """(UTC ms, packets) for each of the echo station's replays."""
    offset = range_report.clock_offset(echo_gps)
    return [(t + offset, int(re.search(r"(\d+) packets", msg).group(1)))
            for t, _, msg in echo if "ECHO replaying" in msg]


def replay_size(replays, utc_start, utc_end):
    """How many packets the echo replayed in this window: it starts replaying just before
    the handheld hears the first copy."""
    return next((n for t, n in replays if utc_start - 4000 <= t <= utc_end), None)


def relayed_windows(hand):
    """(txid, start, end) in log ms for each transmission the handheld heard that had
    relayed copies in it: from its own push-to-talk release (or 3 s before the first
    copy) to the conveyor's end."""
    for i, (t, _, msg) in enumerate(hand):
        m = re.search(r"CONVEYOR start txid=(\d+)", msg)
        if not m:
            continue
        txid = m.group(1)
        end = next(((t2, m2) for t2, _, m2 in hand[i:]
                    if re.search(rf"CONVEYOR (ended|over).*txid={txid} ", m2)), None)
        if not end or not re.search(r"relayed [1-9]", end[1]):
            continue
        released = [t2 for t2, _, m2 in hand[:i] if "PTT released" in m2]
        start = max(t - 3000, released[-1] + 50 if released else 0)
        yield txid, start, end[0] + 50


def read_window(hand, txid, start, end):
    """The packets and IRQs in one window, on a time axis where 0 is the end of the
    repeater's copy of packet 0."""
    lines = [msg for t, _, msg in hand if start <= t <= end]
    times = [t for t, _, msg in hand if start <= t <= end]

    # Packet ends (good or CRC error), with the radio's own stamp. A good one's next
    # line is the app's, saying which packet it was and its hops
    ends = []
    for i, msg in enumerate(lines):
        stamp = re.search(r"at=(\d+)us", msg)
        if not stamp or not ("RX end" in msg or "RX CRC error" in msg):
            continue
        signal = re.search(r"rssi=(-?\d+) snr=(-?\d+)", msg)
        app = PACKET_LINE.search(lines[i + 1]) if "RX end" in msg and i + 1 < len(lines) else None
        ours = app and app.group(1) == txid
        ends.append({
            "at": int(stamp.group(1)) / 1000,
            "log": times[i],
            "rssi": int(signal.group(1)),
            "snr": int(signal.group(2)),
            "ok": "RX end" in msg,
            "packet": int(app.group(2)) if ours else None,
            "hops": int(app.group(3)) if ours else None,
        })
    relayed = [e for e in ends if e["hops"] == 1]
    if not relayed:
        return None

    origin = statistics.median(e["at"] - e["packet"] * SLOT_MS for e in relayed)
    # Log clock minus radio clock: the smallest gap, as a packet's line is logged
    # just after its stamp
    log_offset = min(e["log"] - e["at"] for e in ends)

    irqs = []
    for t, msg in zip(times, lines):
        kind = {"RX preamble": "preamble", "RX header": "header", "RX header error": "header error"}.get(msg)
        if kind:
            # Logged somewhere in the 10 ms tick after the IRQ: put it mid-tick
            irqs.append((t - log_offset - 5 - origin, kind))
    for e in ends:
        e["t"] = e["at"] - origin
        irqs.append((e["t"], "end" if e["ok"] else "crc error"))
    irqs.sort()

    return {
        "txid": txid,
        "ends": ends,
        "irqs": irqs,
        "highest": max(e["packet"] for e in relayed),
    }


# --- drawing ----------------------------------------------------------------


ROW_RELAYED, ROW_IRQS, ROW_DIRECT = 2, 1, 0
IRQ_MARKS = {"preamble": ("v", INK), "header": ("o", INK), "header error": ("X", RED),
             "end": ("|", INK), "crc error": ("|", RED)}


def copies_at(panel, box_end):
    """What was decoded in the copy whose box ends at box_end."""
    return [e for e in panel["ends"] if abs(e["t"] - box_end) < SAME_COPY_MS]


def plot(out, panels):
    plt.rcParams.update({"font.family": "sans-serif", "font.size": 9, "axes.edgecolor": "#c3c2b7",
                         "axes.labelcolor": MUTED, "xtick.color": MUTED, "ytick.color": MUTED})
    fig, axes = plt.subplots(len(panels), 1, figsize=(17, 2.5 * len(panels)), squeeze=False)
    for ax, panel in zip(axes[:, 0], panels):
        draw_copies(ax, panel)
        draw_stuck_spans(ax, panel)
        for t, kind in panel["irqs"]:
            marker, colour = IRQ_MARKS[kind]
            ax.plot([t], [ROW_IRQS], marker=marker, color=colour, linestyle="none", zorder=3,
                    markersize=11 if marker == "|" else 7)
        style_axes(ax, panel)
    axes[-1, 0].set_xlabel("ms (0 = end of the repeater's copy of packet 0; grid = the 80 ms bins)")
    add_legend(fig)
    fig.suptitle("Handheld receiver during the echo's replays via the repeater. "
                 "Labels under the boxes: RSSI dBm / SNR dB", y=1.025, fontsize=11, color=INK)
    fig.tight_layout(rect=(0, 0, 1, 0.97))
    fig.savefig(out, dpi=130, bbox_inches="tight", facecolor=SURFACE)


def draw_copies(ax, panel):
    for packet in range(panel["packets"]):
        relayed_end = packet * SLOT_MS
        for row, box_end, colour in ((ROW_RELAYED, relayed_end, BLUE), (ROW_DIRECT, relayed_end - BIN_MS, ORANGE)):
            decoded = copies_at(panel, box_end)
            good = any(e["ok"] for e in decoded)
            ax.add_patch(Rectangle((box_end - AIR_MS, row - 0.32), AIR_MS, 0.64,
                                   facecolor=colour if good else "none", edgecolor=colour, linewidth=1.2,
                                   alpha=0.9 if good else 0.6, linestyle="-" if good else (0, (3, 2))))
            for e in decoded:
                if not e["ok"]:
                    ax.add_patch(Rectangle((box_end - AIR_MS, row - 0.32), AIR_MS, 0.64,
                                           facecolor="none", edgecolor=RED, linewidth=2))
                ax.text(box_end - AIR_MS / 2, row - 0.5, f"{e['rssi']}/{e['snr']:+d}", ha="center", va="top",
                        fontsize=6.5, color=MUTED if e["ok"] else RED)
            if row == ROW_RELAYED:
                ax.text(box_end - AIR_MS / 2, row + 0.42, str(packet), ha="center", va="bottom", fontsize=7,
                        color=INK if good else RED, fontweight="normal" if good else "bold")


def draw_stuck_spans(ax, panel):
    """A preamble with nothing after it but the next preamble, where a relayed copy
    started inside that span and was never decoded."""
    irqs = panel["irqs"]
    for i, (t, kind) in enumerate(irqs):
        if kind != "preamble":
            continue
        following = irqs[i + 1] if i + 1 < len(irqs) else None
        if following and following[1] != "preamble":
            continue
        until = following[0] if following else t + SLOT_MS
        swallowed = [packet for packet in range(panel["packets"])
                     if t < packet * SLOT_MS - AIR_MS + 8 < until
                     and not any(e["ok"] for e in copies_at(panel, packet * SLOT_MS))]
        if swallowed:
            ax.axvspan(t, until, ymin=0.05, ymax=0.95, color=YELLOW, alpha=0.28, linewidth=0, zorder=0)


def style_axes(ax, panel):
    ax.set_yticks([ROW_RELAYED, ROW_IRQS, ROW_DIRECT],
                  ["relayed copy\n(repeater)", "radio IRQs", "direct copy\n(echo)"])
    ax.set_ylim(-0.95, 2.95)
    ax.set_xlim(-BIN_MS - AIR_MS - 25, (panel["packets"] - 1) * SLOT_MS + 40)
    ax.set_xticks([packet * SLOT_MS for packet in range(panel["packets"])])
    ax.set_xticks([packet * SLOT_MS - BIN_MS for packet in range(panel["packets"] + 1)], minor=True)
    ax.grid(axis="x", which="both", color=GRID, linewidth=0.8)
    ax.set_axisbelow(True)
    ax.tick_params(axis="x", labelsize=7)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    played = len({e["packet"] for e in panel["ends"]
                  if e["packet"] is not None and e["packet"] < panel["packets"]})
    where = f"handheld {panel['from_echo']:.0f} m from echo"
    if panel["from_repeater"] is not None:
        where += f", {panel['from_repeater']:.0f} m from repeater"
    ax.set_title(f"{panel['utc']:%H:%M:%S} UTC   echo replay txid {panel['txid']}, {panel['packets']} packets   "
                 f"{where}   played {played}/{panel['packets']}", loc="left", fontsize=9.5, color=INK)


def add_legend(fig):
    handles = [
        Rectangle((0, 0), 1, 1, fc=BLUE, ec=BLUE),
        Rectangle((0, 0), 1, 1, fc="none", ec=BLUE, ls=(0, (3, 2))),
        Rectangle((0, 0), 1, 1, fc=ORANGE, ec=ORANGE),
        Rectangle((0, 0), 1, 1, fc="none", ec=RED, lw=2),
        Line2D([], [], marker="v", color=INK, ls="none"),
        Line2D([], [], marker="o", color=INK, ls="none"),
        Line2D([], [], marker="X", color=RED, ls="none"),
        Rectangle((0, 0), 1, 1, fc=YELLOW, alpha=0.4, ec="none"),
    ]
    labels = ["copy received", "copy expected, nothing decoded", "direct copy received", "CRC error (garbled)",
              "preamble IRQ (+-5 ms)", "header IRQ", "header error IRQ",
              "preamble, then no header and no error: deaf until the next preamble"]
    fig.legend(handles, labels, loc="upper center", ncol=4, frameon=False, fontsize=8.5, bbox_to_anchor=(0.5, 1.0))


if __name__ == "__main__":
    main()
