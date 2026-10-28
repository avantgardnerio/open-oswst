"""Bench session helper for the automated bench (Jumperless V5 + overhead Arducam).

Runs MicroPython on the Jumperless over its raw REPL, grabs camera stills, and appends both
to a session log in ~/bench-sessions/<session>/log.md, so a whole session can be read back
or shared as it happened.

  bench.py <session> run '<python code>' [timeout_s]   run code on the Jumperless, log code + output
  bench.py <session> snap <name> '<caption>'          full-res still, upright, logged with caption
  bench.py <session> note '<text>'                    free text into the log

Other scripts import run(), snap() and log() from here.
"""
import datetime
import pathlib
import sys
import time

# The Jumperless enumerates 4 USB serial ports: if00 menu, if02 Arduino UART,
# if04 MicroPython REPL, if06 read-only status (docs/jumperless/README.md).
REPL_PORT = "/dev/serial/by-id/usb-Architeuthis_Flux_Jumperless_V5_JLV5port-if04"
SESSIONS = pathlib.Path.home() / "bench-sessions"
# The newest still, full size and small, outside any session folder: an image viewer can
# keep LATEST open (it reloads on change), PREVIEW is small enough to look at quickly.
LATEST = SESSIONS / ".latest.jpg"
PREVIEW = SESSIONS / ".preview.jpg"


def session_dir(session):
    path = SESSIONS / session
    path.mkdir(parents=True, exist_ok=True)
    return path


def log(session, text):
    stamp = datetime.datetime.now().isoformat(timespec="seconds")
    with open(session_dir(session) / "log.md", "a") as log_file:
        log_file.write(f"\n### {stamp}\n\n{text}\n")


def read_until(port, marker, timeout_s):
    data = b""
    deadline = time.time() + timeout_s
    while not data.endswith(marker):
        if time.time() > deadline:
            raise TimeoutError(f"waiting for {marker!r}, got {data[-200:]!r}")
        data += port.read(1)
    return data[: -len(marker)]


def run(code, timeout_s=20):
    # MicroPython raw REPL: Ctrl-A enters, code + Ctrl-D executes, the reply is
    # "OK" <stdout> \x04 <stderr> \x04 ">". Ctrl-B returns to the friendly REPL.
    import serial

    with serial.Serial(REPL_PORT, 115200, timeout=0.1) as port:
        port.write(b"\r\x03\x03")
        time.sleep(0.2)
        port.reset_input_buffer()
        port.write(b"\x01")
        read_until(port, b"raw REPL; CTRL-B to exit\r\n>", 3)
        port.write(code.encode() + b"\x04")
        read_until(port, b"OK", 3)
        stdout = read_until(port, b"\x04", timeout_s).decode(errors="replace")
        stderr = read_until(port, b"\x04", 3).decode(errors="replace")
        read_until(port, b">", 3)
        port.write(b"\x02")
    return stdout, stderr


def snap(path):
    import cv2

    capture = cv2.VideoCapture(0, cv2.CAP_V4L2)
    capture.set(cv2.CAP_PROP_FOURCC, cv2.VideoWriter_fourcc(*"MJPG"))
    capture.set(cv2.CAP_PROP_FRAME_WIDTH, 4032)
    capture.set(cv2.CAP_PROP_FRAME_HEIGHT, 3040)
    # The first frames are dark while auto exposure settles.
    for _ in range(15):
        ok, frame = capture.read()
    capture.release()
    if not ok:
        raise RuntimeError("camera read failed")
    # The camera hangs upside down relative to the user's seat.
    frame = cv2.rotate(frame, cv2.ROTATE_180)
    cv2.imwrite(str(path), frame, [cv2.IMWRITE_JPEG_QUALITY, 92])
    cv2.imwrite(str(LATEST), frame, [cv2.IMWRITE_JPEG_QUALITY, 92])
    height, width = frame.shape[:2]
    cv2.imwrite(str(PREVIEW), cv2.resize(frame, (1600, int(1600 * height / width))))


def main():
    session, action = sys.argv[1], sys.argv[2]
    if action == "run":
        code = sys.argv[3]
        timeout_s = float(sys.argv[4]) if len(sys.argv) > 4 else 20
        stdout, stderr = run(code, timeout_s)
        print(stdout, end="")
        if stderr:
            print("STDERR:", stderr)
        log(session, f"Jumperless:\n\n```python\n{code}\n```\n\nOutput:\n\n```\n{stdout}{stderr}```")
    elif action == "snap":
        name, caption = sys.argv[3], sys.argv[4]
        path = session_dir(session) / f"{name}.jpg"
        snap(path)
        print(path)
        log(session, f"![{caption}]({name}.jpg)\n\n{caption}")
    elif action == "note":
        log(session, sys.argv[3])


if __name__ == "__main__":
    main()
