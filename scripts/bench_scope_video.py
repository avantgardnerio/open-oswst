"""Side-by-side video: the bench camera with an AD3 recording scrolling beside it, in sync.

  bench_scope_video.py <camera.mp4> <ad3.npz> <output.mp4> --title "..." --credit "..." [--credit ...]
                       [--band 300 3400] [--cue START END LABEL ...]

The camera video needs its bench_record.py sidecar (<camera.mp4>.json, start time) and the
.npz its ad3_record.py start time; frames are matched by wall clock. The right panel shows
the last 2 s of the trace (band-passed to the speech band) above a whole-take overview with
a moving cursor. Credits run along the bottom. Output: 1920x1080 H.264.
"""
import argparse
import json
import subprocess

import cv2
import imageio_ffmpeg
import matplotlib
import numpy as np

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

WIDTH, HEIGHT = 1920, 1080
WINDOW_SECONDS = 2.0
BACKGROUND = "#1e2329"
INK = "#e8ecf0"
MUTED = "#96a2ac"
TRACE = "#5fd394"
CURSOR = "#f0a43a"


def band_pass(volts, rate, low, high):
    spectrum = np.fft.rfft(volts - volts.mean())
    frequencies = np.fft.rfftfreq(len(volts), 1 / rate)
    spectrum[(frequencies < low) | (frequencies > high)] = 0
    return np.fft.irfft(spectrum, len(volts))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("camera")
    parser.add_argument("ad3")
    parser.add_argument("output")
    parser.add_argument("--title", required=True)
    parser.add_argument("--credit", action="append", default=[])
    parser.add_argument("--band", nargs=2, type=float, default=[300, 3400])
    parser.add_argument("--cue", nargs=3, action="append", default=[], metavar=("START", "END", "LABEL"),
                        help="seconds from the start of the AD3 recording")
    args = parser.parse_args()

    camera_info = json.loads(open(args.camera + ".json").read())
    recording = np.load(args.ad3)
    rate = float(recording["samples_per_second"])
    trace_mv = band_pass(recording["volts"], rate, *args.band) * 1000
    trace_start = float(recording["start"])
    trace_seconds = np.arange(len(trace_mv)) / rate
    scale_mv = max(3.0, np.percentile(np.abs(trace_mv), 99.9) * 1.2)

    capture = cv2.VideoCapture(args.camera)
    frames_per_second = capture.get(cv2.CAP_PROP_FPS)
    encoder = subprocess.Popen(
        [imageio_ffmpeg.get_ffmpeg_exe(), "-loglevel", "error", "-y",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{WIDTH}x{HEIGHT}", "-r", f"{frames_per_second:.3f}", "-i", "-",
         "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p", "-movflags", "+faststart", args.output],
        stdin=subprocess.PIPE)

    figure = plt.figure(figsize=(WIDTH / 100, HEIGHT / 100), dpi=100, facecolor=BACKGROUND)
    camera_axis = figure.add_axes([0.0, 0.14, 0.62, 0.82])
    window_axis = figure.add_axes([0.655, 0.47, 0.325, 0.42], facecolor=BACKGROUND)
    overview_axis = figure.add_axes([0.655, 0.17, 0.325, 0.2], facecolor=BACKGROUND)
    figure.text(0.012, 0.965, args.title, color=INK, fontsize=24, fontweight="bold", va="center")
    figure.text(0.655, 0.925, f"AD3 scope ch1: mic line, {args.band[0]:.0f}-{args.band[1]:.0f} Hz", color=MUTED, fontsize=15, va="center")
    # One credit per line under the camera, so long ones never run into each other.
    for index, credit in enumerate(args.credit):
        figure.text(0.012, 0.11 - index * 0.035, credit, color=INK, fontsize=15, va="center")
    for axis in (window_axis, overview_axis):
        axis.tick_params(colors=MUTED, labelsize=11)
        for spine in axis.spines.values():
            spine.set_color("#3e4750")
        axis.grid(alpha=0.15)
    camera_axis.axis("off")

    window_axis.set_ylim(-scale_mv, scale_mv)
    window_axis.set_ylabel("mV", color=MUTED, fontsize=12)
    window_line, = window_axis.plot([], [], color=TRACE, lw=0.7)
    cue_text = window_axis.text(0.02, 0.92, "", transform=window_axis.transAxes, color=CURSOR, fontsize=18, fontweight="bold")
    overview_axis.plot(trace_seconds, trace_mv, color=TRACE, lw=0.4)
    overview_axis.set_ylim(-scale_mv, scale_mv)
    overview_axis.set_xlim(0, trace_seconds[-1])
    overview_axis.set_xlabel("s", color=MUTED, fontsize=12)
    for start, end, label in args.cue:
        overview_axis.axvspan(float(start), float(end), color=CURSOR, alpha=0.08)
        overview_axis.text((float(start) + float(end)) / 2, scale_mv * 0.8, label, color=MUTED, fontsize=10, ha="center")
    cursor = overview_axis.axvline(0, color=CURSOR, lw=1.5)
    camera_image = None

    frame_index = 0
    while True:
        ok, frame = capture.read()
        if not ok:
            break
        # Wall-clock time of this frame, as seconds into the AD3 recording.
        now = camera_info["start"] + frame_index / frames_per_second - trace_start
        frame_index += 1
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        if camera_image is None:
            camera_image = camera_axis.imshow(rgb)
        else:
            camera_image.set_data(rgb)
        low = max(0, int((now - WINDOW_SECONDS) * rate))
        high = max(0, min(len(trace_mv), int(now * rate)))
        window_line.set_data(trace_seconds[low:high] - now, trace_mv[low:high])
        window_axis.set_xlim(-WINDOW_SECONDS, 0)
        cursor.set_xdata([min(max(now, 0), trace_seconds[-1])] * 2)
        cue_text.set_text(next((label for start, end, label in args.cue if float(start) <= now < float(end)), ""))
        figure.canvas.draw()
        buffer = np.asarray(figure.canvas.buffer_rgba())[:, :, :3]
        encoder.stdin.write(np.ascontiguousarray(buffer).tobytes())

    encoder.stdin.close()
    encoder.wait()
    print("wrote", args.output, frame_index, "frames")


if __name__ == "__main__":
    main()
