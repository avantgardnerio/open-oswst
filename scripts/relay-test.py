#!/usr/bin/env python3
"""The standard relay metric: handheld → repeater → echo station → back.

Run it, wait ~5s (opening the ports reboots the boards), then do the usual
desk test: PTT on the handheld, talk ~5s, let go, wait for the echo. Repeat
as many times as you like before the capture ends. Every PTT gets the same
report, so each fix can be compared against the last.

Each board's role comes from its boot line `Config: mode=...`, so no MACs or
ports are hardcoded. Logs are kept in out/relay-test/<time>/, and can be
re-reported later with --logs.

Usage:
    python3 scripts/relay-test.py               # capture 120s, then report
    python3 scripts/relay-test.py 60            # capture 60s
    python3 scripts/relay-test.py --logs DIR    # report on a past capture

The metrics, per PTT (txid):
  late TX     For each relay: time from the repeater being back in RX to when
              the talker's next packet should start, assuming the talker keeps
              its 160ms cadence: RX end - AIR_MS + PERIOD_MS - back-in-RX.
              Negative = late: the repeater was deaf when it started.
  relayed     Talker packets the repeater relayed
  recorded    Talker packets the echo station recorded
  echo heard  Replay packets the handheld heard (direct or relayed)
  TX steps    standby / prep / tx / back_to_rx per role, from `TX end` lines
  audio       Handheld speaker gaps, underruns, slow encodes, worst heap alloc

Needs pyserial (system python3 has it).
"""

import os
import re
import statistics
import sys
import threading
import time

import serial
import serial.tools.list_ports

ESP_VID = 0x303A
PERIOD_MS = 160  # one packet of audio
AIR_MS = 61.7  # a 26B packet at SF7/125k, CR4/5

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


def main():
    args = sys.argv[1:]
    if args[:1] == ["--logs"]:
        log_dir = args[1]
    else:
        secs = float(args[0]) if args else 120.0
        log_dir = capture(secs)
    report(load_boards(log_dir))


# --- capture ---------------------------------------------------------------


def capture(secs):
    ports = [p for p in serial.tools.list_ports.comports() if p.vid == ESP_VID]
    if not ports:
        sys.exit("No ESP boards found")
    log_dir = os.path.join(ROOT, "out", "relay-test", time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(log_dir)
    print(f"Capturing {secs:.0f}s from {len(ports)} boards into {log_dir}")
    print("Wait ~5s for the boards to reboot, then PTT on the handheld.")
    threads = [
        threading.Thread(target=read_port, args=(p.device, p.serial_number, log_dir, secs))
        for p in ports
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return log_dir


def read_port(device, mac, log_dir, secs):
    s = serial.Serial()
    s.port, s.baudrate, s.timeout = device, 115200, 0.2
    s.dtr = False
    s.rts = False
    s.open()
    end = time.time() + secs
    buf = b""
    with open(os.path.join(log_dir, mac.replace(":", "") + ".log"), "w") as f:
        while time.time() < end:
            buf += s.read(4096)
            *lines, buf = buf.split(b"\n")
            for line in lines:
                f.write(line.decode(errors="replace").rstrip() + "\n")
    s.close()


# --- parsing ---------------------------------------------------------------

# ESP-IDF log line: `I (12345) target: message`
LINE = re.compile(r"([IWE]) \((\d+)\) [^ ]+: (.*)")


class Board:
    def __init__(self, name, lines):
        self.name = name
        self.lines = lines  # (ms since boot, message)
        self.mode = next(
            (m.group(1) for _, msg in lines if (m := re.match(r"Config: mode=(\w+)", msg))),
            "?",
        )


def load_boards(log_dir):
    boards = []
    for name in sorted(os.listdir(log_dir)):
        lines = []
        for raw in open(os.path.join(log_dir, name), errors="replace"):
            m = LINE.search(raw)
            if m:
                lines.append((int(m.group(2)), m.group(3)))
        boards.append(Board(name.removesuffix(".log"), lines))
    return boards


def by_mode(boards, mode):
    return next((b for b in boards if b.mode == mode), None)


def unique_in_order(seqs):
    """Distinct packets in a stream of 4-bit seqs, in arrival order: a step
    of 1..8 forward is new, anything else is a repeat (e.g. via a relay)."""
    count, last = 0, None
    for seq in seqs:
        if last is None or 1 <= (seq - last) % 16 <= 8:
            count += 1
            last = seq
    return count


TX_END = re.compile(
    r"TX end \[26B\] \d+ms: standby=(\d+)us prep=(\d+)us tx=(\d+)us back_to_rx=(\d+)us"
)
# The radio's µs timestamp; the log's own moves in 10ms ticks
AT_US = re.compile(r" at=(\d+)us")


def at_ms(line):
    """A line's time in ms: its µs `at=` if it has one, else the log's."""
    ms, msg = line
    m = AT_US.search(msg)
    return int(m.group(1)) / 1000 if m else ms


def tx_steps(lines):
    """Per-step TX times in ms, from the `TX end [26B]` lines given."""
    steps = {"standby": [], "prep": [], "tx": [], "back_to_rx": []}
    for _, msg in lines:
        m = TX_END.match(msg)
        if m:
            for key, us in zip(steps, m.groups()):
                steps[key].append(int(us) / 1000)
    return steps


def stats(values):
    if not values:
        return "n/a"
    sd = statistics.pstdev(values) if len(values) > 1 else 0.0
    return (
        f"min {min(values):6.1f}  med {statistics.median(values):6.1f}  "
        f"max {max(values):6.1f}  sd {sd:5.1f}  (n={len(values)})"
    )


# --- the metrics -----------------------------------------------------------


def report(boards):
    handheld = by_mode(boards, "Normal")
    repeater = by_mode(boards, "Repeater")
    echo = by_mode(boards, "Echo")
    for role, board in [("handheld", handheld), ("repeater", repeater), ("echo", echo)]:
        print(f"{role:9} {board.name if board else 'MISSING'}")
    if not (handheld and repeater and echo):
        sys.exit("Need one board in each mode (Normal, Repeater, Echo)")

    for start, txid, sent, after in transmissions(handheld):
        print(f"\n=== PTT txid={txid}: {sent} packets sent ===")
        report_repeater(repeater, txid, sent)
        replayed = report_echo(echo, txid, sent)
        report_handheld(handheld, txid, start, after, replayed)


def transmissions(handheld):
    """(PTT press time, txid, packets sent, lines from release to the next press)"""
    found = []
    lines = handheld.lines
    for i, (ms, msg) in enumerate(lines):
        m = re.match(r"PTT pressed — streaming \(txid=(\d+)\)", msg)
        if not m:
            continue
        release = next(
            (j for j in range(i, len(lines)) if lines[j][1].startswith("PTT released")), None
        )
        if release is None:
            continue
        sent = int(re.search(r"(\d+) packets sent", lines[release][1]).group(1))
        nxt = next(
            (j for j in range(release, len(lines)) if lines[j][1].startswith("PTT pressed")),
            len(lines),
        )
        found.append((i, int(m.group(1)), sent, (release, nxt)))
    return found


def report_repeater(repeater, txid, sent):
    lines = repeater.lines
    margins, relayed_seqs, relay_lines = [], [], []
    for i, (ms, msg) in enumerate(lines):
        m = re.match(rf"RELAY \[26B\] txid={txid} seq=(\d+)", msg)
        if not m:
            continue
        relayed_seqs.append(int(m.group(1)))
        rx_end = next(
            (at_ms(lines[j]) for j in range(i - 1, max(i - 6, -1), -1) if lines[j][1].startswith("RX end [26B]")),
            None,
        )
        done = next(
            (j for j in range(i, min(i + 8, len(lines))) if lines[j][1].startswith("TX end [26B]")),
            None,
        )
        if rx_end is None or done is None:
            continue
        relay_lines.append(lines[done])
        margins.append(rx_end - AIR_MS + PERIOD_MS - at_ms(lines[done]))
    late = sum(1 for m in margins if m < 0)
    print(f"  repeater relayed  {unique_in_order(relayed_seqs)}/{sent}")
    print(f"  late TX margin ms {stats(margins)}  late: {late}")
    for step, values in tx_steps(relay_lines).items():
        print(f"    relay {step:10} {stats(values)}")


def report_echo(echo, txid, sent):
    """Prints what the echo recorded; returns how many packets it replayed."""
    seqs, replayed = [], None
    for _, msg in echo.lines:
        if m := re.match(rf"ECHO rec txid={txid} seq=(\d+)", msg):
            seqs.append(int(m.group(1)))
        elif seqs and (m := re.match(r"ECHO replaying (\d+) packets", msg)):
            replayed = int(m.group(1))
            break
    print(f"  echo recorded     {unique_in_order(seqs)}/{sent}, replayed {replayed}")
    return replayed


def report_handheld(handheld, txid, start, after, replayed):
    lines = handheld.lines
    release, nxt = after
    steps = tx_steps(lines[start:release + 3])
    for step, values in steps.items():
        print(f"    talker {step:9} {stats(values)}")
    slow = sum(1 for _, msg in lines[start:release + 3] if "TX encode+send took" in msg)

    # After release: the echo's replay, under a txid that isn't ours (ours
    # still arrives for a moment, relayed back by the repeater)
    window = lines[release:nxt]
    echo_seqs = [
        int(m.group(2))
        for _, msg in window
        if (m := re.match(r"RX \[26B\] txid=(\d+) seq=(\d+)", msg)) and int(m.group(1)) != txid
    ]
    gaps = sum(1 for _, msg in window if msg.startswith("SPK gap"))
    underruns = sum(1 for _, msg in window if msg.startswith("SPK underrun"))
    allocs = [
        int(m.group(1))
        for _, msg in window
        if (m := re.match(r"Audio heap alloc: worst (\d+)us", msg))
    ]
    print(
        f"  echo heard        {unique_in_order(echo_seqs)}/{replayed}, "
        f"{gaps} speaker gaps, {underruns} underruns"
    )
    print(f"  talker slow encodes {slow}, worst audio heap alloc {f"{max(allocs)}us" if allocs else "n/a"}")


if __name__ == "__main__":
    main()
