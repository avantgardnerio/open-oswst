"""Record the overhead camera to MP4 until a stop file appears (or a time limit).

  bench_record.py <output.mp4> <stop-file> [max_seconds]

Frames go straight to disk as they arrive (1080p is ~6 MB per frame raw, too much to hold).
The file is MPEG-4 Part 2, which browsers and LinkedIn won't play: bench_video.py re-encodes
to H.264 for sharing.
"""
import pathlib
import sys
import time

import cv2

output_path, stop_path = sys.argv[1], pathlib.Path(sys.argv[2])
max_seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 180
stop_path.unlink(missing_ok=True)

capture = cv2.VideoCapture(0, cv2.CAP_V4L2)
capture.set(cv2.CAP_PROP_FOURCC, cv2.VideoWriter_fourcc(*"MJPG"))
capture.set(cv2.CAP_PROP_FRAME_WIDTH, 1920)
capture.set(cv2.CAP_PROP_FRAME_HEIGHT, 1080)
capture.set(cv2.CAP_PROP_FPS, 30)
# Let auto exposure settle, and time 30 frames so the file plays back in real time
# (the Arducam delivers ~22 fps at 1080p, not the 30 asked for).
for _ in range(15):
    capture.read()
probe_start = time.time()
for _ in range(30):
    ok, frame = capture.read()
frames_per_second = 30 / (time.time() - probe_start)
height, width = frame.shape[:2]
writer = cv2.VideoWriter(output_path, cv2.VideoWriter_fourcc(*"mp4v"), frames_per_second, (width, height))

frame_count = 0
start = time.time()
while not stop_path.exists() and time.time() - start < max_seconds:
    ok, frame = capture.read()
    if ok:
        # The camera hangs upside down relative to the user's seat.
        writer.write(cv2.rotate(frame, cv2.ROTATE_180))
        frame_count += 1
elapsed = time.time() - start
capture.release()
writer.release()
print(f"{frame_count} frames, {elapsed:.1f} s, {frame_count / elapsed:.1f} fps (file says {frames_per_second:.1f}) -> {output_path}")
