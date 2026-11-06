#!/usr/bin/env python3
"""Record the mic_stream bin's USB stream to a WAV (8 kHz, 16-bit) plus an .npz.

    .venv/bin/python scripts/mic_stream.py <port> <output.wav> [seconds] [cue ...]

A cue is "<seconds>:<text>", shown on the radio's OLED at that time after the first frame; '|' in
the text starts a new OLED line. "DONE" is shown when the recording ends. Example:
    mic_stream.py <port> take.wav 15 "0:QUIET" "5:TALK|NO PTT" "13:QUIET"

Opening the port resets the board (Linux raises DTR/RTS on open, even with pyserial told not to),
so every take starts with ~0.5 s of boot log, which is skipped; the stream starts at frame 0. The .npz next to
the WAV holds raw (12-bit ADC counts), pcm, ptt (1 = pressed) and frame_numbers. Prints the
dropped frames, the PTT presses and the levels, so a take can be judged without opening a file.
"""

import sys
import time
import wave

import numpy as np
import serial

SAMPLE_RATE = 8000
FRAME_SAMPLES = 320


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    port = sys.argv[1]
    output = sys.argv[2]
    seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 10.0
    cues = []
    for argument in sys.argv[4:]:
        at, text = argument.split(":", 1)
        cues.append((float(at), text))
    cues.sort()

    connection = serial.Serial()
    connection.port = port
    connection.baudrate = 115200
    connection.timeout = 0.5
    connection.dtr = False
    connection.rts = False
    connection.open()
    connection.reset_input_buffer()

    frame_numbers = []
    words = []
    other_lines = 0
    cue_log = []  # (sample index, text)
    first_frame_time = None
    end = time.time() + seconds
    while time.time() < end:
        if first_frame_time is not None and cues and time.time() - first_frame_time >= cues[0][0]:
            text = cues.pop(0)[1]
            connection.write(f"CUE {text}\n".encode())
            cue_log.append((len(words), text))
        line = connection.readline().decode(errors="replace").strip()
        parts = line.split(" ")
        if len(parts) != 3 or parts[0] != "PCM" or len(parts[2]) != FRAME_SAMPLES * 4:
            if line:
                other_lines += 1
            continue
        if first_frame_time is None:
            first_frame_time = time.time()
        frame_numbers.append(int(parts[1]))
        words.extend(int(parts[2][i : i + 4], 16) for i in range(0, len(parts[2]), 4))
    connection.write(b"CUE DONE\n")
    connection.flush()
    connection.close()

    if not frame_numbers:
        sys.exit("no PCM frames: is mic_stream running on that port?")

    word_array = np.array(words, dtype=np.uint16)
    ptt = (word_array & 1).astype(np.uint8)
    pcm = (word_array & ~np.uint16(1)).astype(np.int16)
    raw = pcm.astype(np.int32) // 16 + 2048
    frame_array = np.array(frame_numbers)
    dropped = int(np.sum(np.diff(frame_array) - 1))

    with wave.open(output, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        wav.writeframes(pcm.tobytes())
    np.savez(
        output.rsplit(".", 1)[0] + ".npz",
        raw=raw,
        pcm=pcm,
        ptt=ptt,
        frame_numbers=frame_array,
        cue_samples=np.array([index for index, _ in cue_log], dtype=np.int64),
        cue_texts=np.array([text for _, text in cue_log]),
    )

    duration = len(pcm) / SAMPLE_RATE
    print(f"{len(frame_numbers)} frames ({duration:.1f} s), dropped {dropped}, other lines {other_lines}")
    print(f"raw counts: mean {raw.mean():.0f}, min {raw.min()}, max {raw.max()}, ac rms {raw.std():.1f}")
    edges = np.flatnonzero(np.diff(ptt.astype(np.int8)))
    presses = [f"{'down' if ptt[i + 1] else 'up'} {(i + 1) / SAMPLE_RATE:.2f} s" for i in edges]
    print("cues: " + (", ".join(f"{index / SAMPLE_RATE:.2f} s {text}" for index, text in cue_log) or "none"))
    print(f"PTT: {'held at start' if ptt[0] else 'up at start'}; " + (", ".join(presses) or "no changes"))
    # Level per second, so talk and quiet stretches show without a plot
    per_second = [raw[i : i + SAMPLE_RATE].std() for i in range(0, len(raw), SAMPLE_RATE)]
    print("ac rms per second (counts): " + " ".join(f"{level:.0f}" for level in per_second))


if __name__ == "__main__":
    main()
