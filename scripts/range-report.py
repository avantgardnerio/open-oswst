#!/usr/bin/env python3
"""Range report for an echo test: one row per transmission, and a map.

Joins a handheld's log with the echo station's log (both pulled with
pull-logs.py). For each time the handheld keyed up, it finds:
  - where the handheld was (its GPS lines) and how far from the station,
  - outbound: how many packets the echo station heard, at what RSSI/SNR,
  - return: how many echo packets the handheld's radio got, and how many
    the app actually played.
The two boards' clocks are lined up using the UTC in their GPS lines.

Prints the table, and writes next to the handheld log:
  map.html      grey basemap (OpenStreetMap and satellite in the layer switcher)
                with a circle per transmission, labelled with its distance:
                fill = outbound, outline = return; click for the numbers
  walk.geojson  the same points plus the track, for QGIS and friends

Usage:
    .venv/bin/python scripts/range-report.py HANDHELD_LOG ECHO_LOG
e.g.
    .venv/bin/python scripts/range-report.py \\
        logs/F85B1BA73890-20260930-154831/log/0009.txt \\
        logs/F85B1BA2C62C-20260930-154831/log/0013.txt

Needs folium (in requirements.txt).
"""

import datetime as dt
import json
import math
import re
import statistics
import sys
from pathlib import Path

import folium

LINE = re.compile(r"^[EWIDV] \((\d+)\) (\S+): (.*)$")
GPS = re.compile(
    r"GPS (\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ) (?:(-?[\d.]+),(-?[\d.]+)|no-fix) sats=(\d+)"
)

# Share of packets that made it: at or above GOOD is green, at or above FAIR amber
GOOD, FAIR = 0.9, 0.6
COLOURS = {"good": "#1a9641", "fair": "#f4a11d", "bad": "#d7191c", "none": "#888888"}


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    hand_path, echo_path = Path(sys.argv[1]), Path(sys.argv[2])
    hand, echo = parse(hand_path), parse(echo_path)
    hand_gps, echo_gps = gps_points(hand), gps_points(echo)
    if not hand_gps or not echo_gps:
        sys.exit("Both logs need GPS lines (with UTC) to be lined up")

    station = median_position(echo_gps)
    rows = transmissions(hand, echo, hand_gps, echo_gps, station)
    print_table(station, rows)

    out = hand_path.parent
    track = [p["pos"] for p in hand_gps if p["pos"]]
    write_map(out / "map.html", station, track, rows)
    write_geojson(out / "walk.geojson", station, track, rows)
    print(f"\nmap: {out / 'map.html'}\ngeojson: {out / 'walk.geojson'}")


# --- reading the logs ------------------------------------------------------


def parse(path):
    """(uptime_ms, target, message) for each log line."""
    lines = []
    for raw in open(path, errors="replace"):
        m = LINE.match(raw.strip())
        if m:
            lines.append((int(m.group(1)), m.group(2), m.group(3)))
    return lines


def gps_points(lines):
    points = []
    for t, _, msg in lines:
        m = GPS.search(msg)
        if m:
            utc = dt.datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%SZ").replace(
                tzinfo=dt.timezone.utc
            )
            pos = (float(m.group(2)), float(m.group(3))) if m.group(2) else None
            points.append({"t": t, "utc": utc, "pos": pos})
    return points


def clock_offset(points):
    """UTC ms minus uptime ms, for turning a log timestamp into UTC."""
    return statistics.median(p["utc"].timestamp() * 1000 - p["t"] for p in points)


def median_position(points):
    fixes = [p["pos"] for p in points if p["pos"]]
    return (statistics.median(f[0] for f in fixes), statistics.median(f[1] for f in fixes))


def position_at(points, t):
    fixes = [p for p in points if p["pos"]]
    return min(fixes, key=lambda p: abs(p["t"] - t))["pos"] if fixes else None


def distance_m(a, b):
    lat1, lon1, lat2, lon2 = map(math.radians, (*a, *b))
    h = math.sin((lat2 - lat1) / 2) ** 2 + math.cos(lat1) * math.cos(lat2) * math.sin((lon2 - lon1) / 2) ** 2
    return 2 * 6371000 * math.asin(math.sqrt(h))


# --- joining them -----------------------------------------------------------


def transmissions(hand, echo, hand_gps, echo_gps, station):
    hand_off, echo_off = clock_offset(hand_gps), clock_offset(echo_gps)
    # Echo station events, on the UTC clock
    recorded = [(t + echo_off, msg) for t, _, msg in echo if "ECHO rec" in msg]
    replays = [(t + echo_off, int(re.search(r"(\d+) packets", msg).group(1)))
               for t, _, msg in echo if "ECHO replaying" in msg]

    rows = []
    for i, (t, _, msg) in enumerate(hand):
        m = re.search(r"PTT pressed .*txid=(\d+)", msg)
        if not m:
            continue
        txid = int(m.group(1))
        released = next(((t2, int(re.search(r"(\d+) packets", m2).group(1)))
                         for t2, _, m2 in hand[i:] if "PTT released" in m2), None)
        if not released or released[1] == 0:
            continue
        t_end, sent = released
        utc_start, utc_end = t + hand_off, t_end + hand_off

        # Outbound: what the echo station recorded from this transmission
        heard = [stats(mm) for tt, mm in recorded
                 if f"txid={txid} " in mm and utc_start - 2000 <= tt <= utc_end + 2000]
        echoed = next((n for tt, n in replays if utc_start <= tt <= utc_end + 3000), None)

        # Return: what the handheld got in the replay window (1s pause + replay + slack)
        window_end = t_end + 1500 + (echoed or sent) * 160 + 2000
        window = [mm for tt, _, mm in hand if t_end <= tt <= window_end]
        radio_got = sum(1 for mm in window if mm.startswith("RX end [26B]"))
        played = [stats(mm) for mm in window
                  if mm.startswith("RX [26B]") and f"txid={txid} " not in mm]

        pos = position_at(hand_gps, t)
        rows.append({
            "utc": dt.datetime.fromtimestamp(utc_start / 1000, dt.timezone.utc),
            "pos": pos,
            "dist": distance_m(pos, station) if pos else None,
            "sent": sent,
            "heard": len(heard),
            "out_rssi": mean(r for r, _ in heard),
            "out_snr": mean(s for _, s in heard),
            "echoed": echoed,
            "radio_got": radio_got,
            "played": len(played),
            "back_rssi": mean(r for r, _ in played),
            "back_snr": mean(s for _, s in played),
        })
    return rows


def stats(msg):
    m = re.search(r"rssi=(-?\d+) snr=(-?\d+)", msg)
    return int(m.group(1)), int(m.group(2))


def mean(values):
    values = list(values)
    return statistics.mean(values) if values else None


def grade(got, of):
    """good / fair / bad / none for `got` packets out of `of`."""
    if not of:
        return "none"
    share = got / of
    return "good" if share >= GOOD else "fair" if share >= FAIR else "bad"


# --- output -----------------------------------------------------------------


def print_table(station, rows):
    fmt = lambda v, w=5: f"{v:{w}.0f}" if v is not None else " " * (w - 1) + "-"
    print(f"echo station at {station[0]:.5f},{station[1]:.5f}")
    print(f"{'UTC':8} {'dist m':>6} | {'sent':>4} {'heard':>5} {'rssi':>5} {'snr':>4} |"
          f" {'echo':>4} {'radio':>5} {'played':>6} {'rssi':>5} {'snr':>4}")
    for r in rows:
        print(f"{r['utc']:%H:%M:%S} {fmt(r['dist'], 6)} | {r['sent']:4} {r['heard']:5} "
              f"{fmt(r['out_rssi'])} {fmt(r['out_snr'], 4)} | {r['echoed'] or '-':>4} "
              f"{r['radio_got']:5} {r['played']:6} {fmt(r['back_rssi'])} {fmt(r['back_snr'], 4)}")


def popup(r):
    fmt = lambda v: f"{v:.0f}" if v is not None else "-"
    dist = f"{r['dist']:.0f} m" if r["dist"] is not None else "?"
    return (
        f"<b>{r['utc']:%H:%M:%S} UTC</b>, {dist} from the station<br>"
        f"<b>Out</b> (station heard you): {r['heard']}/{r['sent']}, "
        f"RSSI {fmt(r['out_rssi'])}, SNR {fmt(r['out_snr'])}<br>"
        f"<b>Back</b> (you heard the echo): played {r['played']}/{r['echoed'] or '?'}, "
        f"radio got {r['radio_got']}, RSSI {fmt(r['back_rssi'])}, SNR {fmt(r['back_snr'])}"
    )


LEGEND = f"""
<div style="position: fixed; bottom: 24px; left: 24px; z-index: 1000; background: white;
            padding: 8px 12px; border-radius: 6px; font: 13px sans-serif; box-shadow: 0 1px 4px #0005">
  <b>Each circle is one transmission</b><br>
  Fill: the echo station heard you &nbsp;·&nbsp; Outline: you heard the echo<br>
  <span style="color:{COLOURS['good']}">●</span> ≥{GOOD:.0%} of packets &nbsp;
  <span style="color:{COLOURS['fair']}">●</span> ≥{FAIR:.0%} &nbsp;
  <span style="color:{COLOURS['bad']}">●</span> less &nbsp;
  <span style="color:{COLOURS['none']}">●</span> no echo
</div>
"""


# Distance label beside each circle: bold, with a white halo to read on any basemap
LABEL_STYLE = ("font: bold 13px sans-serif; color: #111; white-space: nowrap; "
               "text-shadow: -1px -1px 0 #fff, 1px -1px 0 #fff, -1px 1px 0 #fff, 1px 1px 0 #fff, 0 0 3px #fff")


def write_map(path, station, track, rows):
    # Grey first, so the coloured circles stand out: OpenStreetMap run through a
    # CSS filter (CARTO's ready-made grey tiles now need an API key). Colour
    # streets and satellite are one click away in the layer switcher.
    m = folium.Map(location=station, zoom_start=16, tiles=None)
    folium.TileLayer("OpenStreetMap", name="Grey", class_name="basemap-grey").add_to(m)
    folium.TileLayer("OpenStreetMap", name="Streets").add_to(m)
    m.get_root().header.add_child(folium.Element(
        "<style>.basemap-grey { filter: grayscale(1) contrast(0.8) brightness(1.1); }</style>"))
    folium.TileLayer(
        tiles="https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
        attr="Esri World Imagery",
        name="Satellite",
    ).add_to(m)

    if track:
        folium.PolyLine(track, color="#3366cc", weight=2, opacity=0.6, tooltip="walk").add_to(m)
    folium.Marker(station, tooltip="echo station",
                  icon=folium.Icon(color="blue", icon="home", prefix="fa")).add_to(m)

    for r in rows:
        if not r["pos"]:
            continue
        # Thin dark ring underneath, so the circle stands out on satellite too
        folium.CircleMarker(r["pos"], radius=16, color="#222", weight=1.5, fill=False).add_to(m)
        folium.CircleMarker(
            r["pos"],
            radius=13,
            fill=True,
            fill_color=COLOURS[grade(r["heard"], r["sent"])],
            fill_opacity=1.0,
            color=COLOURS[grade(r["played"], r["echoed"])],
            weight=5,
            tooltip=f"{r['utc']:%H:%M:%S}",
            popup=folium.Popup(popup(r), max_width=320),
        ).add_to(m)
        if r["dist"] is not None:
            folium.Marker(
                r["pos"],
                icon=folium.DivIcon(
                    html=f'<div style="{LABEL_STYLE}">{r["dist"]:.0f} m</div>',
                    icon_anchor=(-19, 9),  # just right of the circle, vertically centred
                ),
            ).add_to(m)

    folium.LayerControl().add_to(m)
    m.get_root().html.add_child(folium.Element(LEGEND))
    if track:
        m.fit_bounds(track + [station])
    m.save(str(path))


def write_geojson(path, station, track, rows):
    features = [{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [station[1], station[0]]},
        "properties": {"kind": "echo station"},
    }]
    if track:
        features.append({
            "type": "Feature",
            "geometry": {"type": "LineString", "coordinates": [[lon, lat] for lat, lon in track]},
            "properties": {"kind": "walk"},
        })
    for r in rows:
        if not r["pos"]:
            continue
        props = {k: v for k, v in r.items() if k not in ("pos", "utc")}
        props.update(kind="transmission", utc=r["utc"].isoformat(),
                     outbound=grade(r["heard"], r["sent"]), back=grade(r["played"], r["echoed"]))
        features.append({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [r["pos"][1], r["pos"][0]]},
            "properties": props,
        })
    path.write_text(json.dumps({"type": "FeatureCollection", "features": features}, indent=1))


if __name__ == "__main__":
    main()
