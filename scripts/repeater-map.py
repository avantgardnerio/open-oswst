#!/usr/bin/env python3
"""Range report for an echo walk WITH a repeater: direct and relayed copies apart.

range-report.py averages every copy a radio heard; with a repeater up, that
mixes the talker's own (direct) copy with the repeater's (relayed) one, and
past the direct link's edge it reports the repeater's signal as if it were
ours. Every packet says its hops (0 = direct, 1 = relayed), so here they're
counted and averaged apart, both ways:
  out   the echo station hearing the handheld
  back  the handheld hearing the echo's replay
Per packet, the first copy decoded is the one logged (the app plays the
first copy of each packet number).

Prints a table, and writes map-NNNN-repeater.html next to the handheld log
(NNNN = its number): a circle per transmission, fill = out, outline = back
(range-report.py's colours), a purple ring when the repeater's copy carried
it, the echo station and the repeater marked; click a circle for the numbers.

Usage:
    .venv/bin/python scripts/repeater-map.py HANDHELD_LOG ECHO_LOG REPEATER_LAT,LON
e.g.
    .venv/bin/python scripts/repeater-map.py \\
        logs/<HANDHELD_MAC>-<time>/log/0010.txt \\
        logs/<ECHO_MAC>-<time>/log/0005.txt <LAT>,<LON>
"""

import datetime as dt
import importlib.util
import re
import statistics
import sys
from pathlib import Path

import folium

# range-report.py has the log parsing, GPS clock alignment and map styling;
# its name has a dash, so it's loaded by path
spec = importlib.util.spec_from_file_location("range_report", Path(__file__).parent / "range-report.py")
range_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(range_report)

PACKET_LINE = re.compile(r"txid=(\d+) packet=(\d+) hops=(\d) rssi=(-?\d+) snr=(-?\d+)")
REPEATER_COLOUR = "#7b3294"


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    hand_path, echo_path = Path(sys.argv[1]), Path(sys.argv[2])
    repeater = tuple(map(float, sys.argv[3].split(",")))

    hand, echo = range_report.parse(hand_path), range_report.parse(echo_path)
    hand_gps, echo_gps = range_report.gps_points(hand), range_report.gps_points(echo)
    station = range_report.median_position(echo_gps)
    rows = transmissions(hand, echo, hand_gps, echo_gps, station, repeater)
    print_table(station, repeater, rows)

    track = [p["pos"] for p in hand_gps if p["pos"]]
    out = hand_path.parent / f"map-{hand_path.stem}-repeater.html"
    write_map(out, station, repeater, track, rows)
    print(f"\nmap: {out}")


# --- joining the logs ---------------------------------------------------------


def transmissions(hand, echo, hand_gps, echo_gps, station, repeater):
    hand_offset, echo_offset = range_report.clock_offset(hand_gps), range_report.clock_offset(echo_gps)
    rows = []
    for i, (t, _, msg) in enumerate(hand):
        m = re.search(r"PTT pressed .*txid=(\d+)", msg)
        if not m:
            continue
        txid = m.group(1)
        released = next(((t2, int(re.search(r"(\d+) packets", m2).group(1)))
                         for t2, _, m2 in hand[i:] if "PTT released" in m2), None)
        if not released or released[1] == 0:
            continue
        t_end, sent = released
        utc_start, utc_end = t + hand_offset, t_end + hand_offset

        # Out: what the echo station recorded of this transmission. Its window
        # runs past the release: relayed copies come a bin late
        echo_window = [(t2 + echo_offset, m2) for t2, _, m2 in echo
                       if utc_start - 2000 <= t2 + echo_offset <= utc_end + 6000]
        out = [PACKET_LINE.search(m2) for _, m2 in echo_window if "ECHO rec" in m2 and f"txid={txid} " in m2]
        replayed = next((int(re.search(r"(\d+) packets", m2).group(1))
                         for _, m2 in echo_window if "ECHO replaying" in m2), None)

        # Back: the echo's replay, in the handheld's next 12 s
        back = [PACKET_LINE.search(m2) for t2, _, m2 in hand
                if t_end <= t2 <= t_end + 12000 and m2.startswith("RX [26B]") and f"txid={txid} " not in m2]

        pos = range_report.position_at(hand_gps, t)
        rows.append({
            "utc": dt.datetime.fromtimestamp(utc_start / 1000, dt.timezone.utc),
            "txid": txid,
            "pos": pos,
            "from_echo": range_report.distance_m(pos, station) if pos else None,
            "from_repeater": range_report.distance_m(pos, repeater) if pos else None,
            "sent": sent,
            "replayed": replayed,
            "out_direct": copies(out, 0),
            "out_relayed": copies(out, 1),
            "back_direct": copies(back, 0),
            "back_relayed": copies(back, 1),
        })
    return rows


def copies(matches, hops):
    """(rssi, snr) of each packet whose logged copy had these hops."""
    return [(int(m.group(4)), int(m.group(5))) for m in matches if m and int(m.group(3)) == hops]


def mean_of(pairs, index):
    values = [pair[index] for pair in pairs]
    return statistics.mean(values) if values else None


# --- output -------------------------------------------------------------------


def print_table(station, repeater, rows):
    fmt = lambda v, w=4: f"{v:{w}.0f}" if v is not None else " " * (w - 1) + "-"
    print(f"echo station at {station[0]:.5f},{station[1]:.5f}; repeater {range_report.distance_m(station, repeater):.0f} m away")
    print(f"{'UTC':8} {'echo m':>6} {'rep m':>5} {'sent':>4} | out direct  rssi  snr | relayed  rssi  snr"
          f" || {'echo':>4} back direct  rssi  snr | relayed  rssi  snr")
    for r in rows:
        cells = []
        for direct, relayed in ((r["out_direct"], r["out_relayed"]), (r["back_direct"], r["back_relayed"])):
            cells.append(f"{len(direct):10} {fmt(mean_of(direct, 0), 5)} {fmt(mean_of(direct, 1))} | "
                         f"{len(relayed):7} {fmt(mean_of(relayed, 0), 5)} {fmt(mean_of(relayed, 1))}")
        print(f"{r['utc']:%H:%M:%S} {fmt(r['from_echo'], 6)} {fmt(r['from_repeater'], 5)} {r['sent']:4} | "
              f"{cells[0]} || {r['replayed'] or '-':>4} {cells[1]}")


def popup(r):
    fmt = lambda v: f"{v:.0f}" if v is not None else "-"
    copy = lambda pairs: f"{len(pairs)} (RSSI {fmt(mean_of(pairs, 0))}, SNR {fmt(mean_of(pairs, 1))})"
    heard = len(r["out_direct"]) + len(r["out_relayed"])
    played = len(r["back_direct"]) + len(r["back_relayed"])
    return (
        f"<b>{r['utc']:%H:%M:%S} UTC</b> txid {r['txid']}<br>"
        f"{r['from_echo']:.0f} m from the echo, {r['from_repeater']:.0f} m from the repeater<br>"
        f"<b>Out</b> echo heard {heard}/{r['sent']}: direct {copy(r['out_direct'])}, relayed {copy(r['out_relayed'])}<br>"
        f"<b>Back</b> played {played}/{r['replayed'] or '-'}: direct {copy(r['back_direct'])}, relayed {copy(r['back_relayed'])}"
    )


def write_map(path, station, repeater, track, rows):
    # Same basemaps as range-report.py: grey first, so the circles stand out
    m = folium.Map(location=station, zoom_start=15, tiles=None)
    folium.TileLayer("OpenStreetMap", name="Grey", class_name="basemap-grey").add_to(m)
    folium.TileLayer("OpenStreetMap", name="Streets").add_to(m)
    m.get_root().header.add_child(folium.Element(
        "<style>.basemap-grey { filter: grayscale(1) contrast(0.8) brightness(1.1); }</style>"))
    folium.TileLayer(
        tiles="https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
        attr="Esri World Imagery",
        name="Satellite",
    ).add_to(m)

    folium.PolyLine(track, color="#3366cc", weight=2, opacity=0.6, tooltip="walk").add_to(m)
    folium.Marker(station, tooltip="echo station",
                  icon=folium.Icon(color="blue", icon="home", prefix="fa")).add_to(m)
    between = range_report.distance_m(station, repeater)
    folium.Marker(repeater, tooltip=f"repeater, {between:.0f} m from the echo",
                  icon=folium.Icon(color="purple", icon="tower-broadcast", prefix="fa")).add_to(m)
    folium.PolyLine([station, repeater], color=REPEATER_COLOUR, weight=2, dash_array="6 6",
                    tooltip=f"echo - repeater {between:.0f} m").add_to(m)

    for r in rows:
        if not r["pos"]:
            continue
        heard = len(r["out_direct"]) + len(r["out_relayed"])
        played = len(r["back_direct"]) + len(r["back_relayed"])
        relay_carried = (len(r["out_relayed"]) > len(r["out_direct"])
                         or len(r["back_relayed"]) > len(r["back_direct"]))
        # Ring underneath: purple when the repeater's copy carried it, else the thin dark one
        folium.CircleMarker(r["pos"], radius=16, fill=False,
                            color=REPEATER_COLOUR if relay_carried else "#222",
                            weight=3 if relay_carried else 1.5).add_to(m)
        folium.CircleMarker(
            r["pos"],
            radius=13,
            fill=True,
            fill_color=range_report.COLOURS[range_report.grade(heard, r["sent"])],
            fill_opacity=1.0,
            color=range_report.COLOURS[range_report.grade(played, r["replayed"])],
            weight=5,
            tooltip=f"{r['utc']:%H:%M:%S}",
            popup=folium.Popup(popup(r), max_width=360),
        ).add_to(m)
        folium.Marker(r["pos"], icon=folium.DivIcon(
            html=f'<div style="{range_report.LABEL_STYLE}">{r["from_echo"]:.0f} m</div>',
            icon_anchor=(-19, 9))).add_to(m)

    folium.LayerControl().add_to(m)
    m.get_root().html.add_child(folium.Element(LEGEND))
    m.fit_bounds(track + [station, repeater])
    m.save(str(path))


LEGEND = f"""
<div style="position: fixed; bottom: 24px; left: 24px; z-index: 1000; background: white;
            padding: 8px 12px; border-radius: 6px; font: 13px sans-serif; box-shadow: 0 1px 4px #0005">
  <b>Each circle is one transmission</b><br>
  Fill: the echo station heard you &nbsp;·&nbsp; Outline: you heard the echo<br>
  <span style="color:{range_report.COLOURS['good']}">●</span> ≥{range_report.GOOD:.0%} of packets &nbsp;
  <span style="color:{range_report.COLOURS['fair']}">●</span> ≥{range_report.FAIR:.0%} &nbsp;
  <span style="color:{range_report.COLOURS['bad']}">●</span> less &nbsp;
  <span style="color:{range_report.COLOURS['none']}">●</span> no echo<br>
  <span style="color:{REPEATER_COLOUR}">◯</span> purple ring: the repeater's copy carried it
</div>
"""


if __name__ == "__main__":
    main()
