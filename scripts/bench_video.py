"""Cut a shareable video from bench recordings: captioned clips, then an optional end card.

  bench_video.py <output.mp4> --clip <video> "<title>" "<subtitle>" [--clip ...] [--end-card <image>]

Each clip gets its caption in the top-left corner and holds its last frame for a moment; the
end card fades in from the last clip and holds for 6 s. Output is 1280x720 H.264 with the
index at the front, which LinkedIn and browsers play (the recorder writes MPEG-4 Part 2,
which they don't). Uses the ffmpeg bundled with imageio-ffmpeg.
"""
import argparse
import subprocess

import cv2
import imageio_ffmpeg
import numpy as np
from PIL import Image, ImageDraw, ImageFont

WIDTH, HEIGHT = 1280, 720
FONT_DIR = "/usr/share/fonts/truetype/dejavu/"
CAPTION_TITLE = ImageFont.truetype(FONT_DIR + "DejaVuSansCondensed-Bold.ttf", 34)
CAPTION_SUBTITLE = ImageFont.truetype(FONT_DIR + "DejaVuSansCondensed-Bold.ttf", 24)
INK = (232, 236, 240)
AMBER = (240, 164, 58)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output")
    parser.add_argument("--clip", nargs=3, action="append", required=True, metavar=("VIDEO", "TITLE", "SUBTITLE"))
    parser.add_argument("--end-card", help="image shown at the end, e.g. from pcb/k1/plot_pins.py")
    args = parser.parse_args()

    # Play back at the recordings' own rate (~22 fps from the Arducam at 1080p).
    frames_per_second = cv2.VideoCapture(args.clip[0][0]).get(cv2.CAP_PROP_FPS)
    encoder = subprocess.Popen(
        [imageio_ffmpeg.get_ffmpeg_exe(), "-loglevel", "error", "-y",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{WIDTH}x{HEIGHT}", "-r", f"{frames_per_second:.3f}", "-i", "-",
         "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p",
         "-movflags", "+faststart", args.output],
        stdin=subprocess.PIPE)

    def emit(image, count=1):
        data = np.asarray(image.convert("RGB")).tobytes()
        for _ in range(count):
            encoder.stdin.write(data)

    last_frame = None
    for video, title, subtitle in args.clip:
        capture = cv2.VideoCapture(video)
        while True:
            ok, frame = capture.read()
            if not ok:
                break
            frame = cv2.resize(frame, (WIDTH, HEIGHT), interpolation=cv2.INTER_AREA)
            image = Image.fromarray(cv2.cvtColor(frame, cv2.COLOR_BGR2RGB))
            draw = ImageDraw.Draw(image, "RGBA")
            draw.rounded_rectangle((24, 24, 664, 120), 10, fill=(20, 24, 28, 210))
            draw.text((44, 34), title, font=CAPTION_TITLE, fill=INK)
            draw.text((44, 80), subtitle, font=CAPTION_SUBTITLE, fill=AMBER)
            emit(image)
            last_frame = image
        # Hold the finished state long enough to take in.
        emit(last_frame, int(frames_per_second * 1.5))

    if args.end_card:
        card = Image.open(args.end_card).convert("RGB").resize((WIDTH, HEIGHT))
        fade_frames = int(frames_per_second * 0.8)
        for step in range(fade_frames):
            emit(Image.blend(last_frame.convert("RGB"), card, (step + 1) / fade_frames))
        emit(card, int(frames_per_second * 6))

    encoder.stdin.close()
    encoder.wait()
    print("wrote", args.output)


if __name__ == "__main__":
    main()
