#!/usr/bin/env python3
"""Continuity map with the Analog Discovery 3: which of its DIO lines are
wired to which. Drives one line high at a time (the rest are inputs, which
read 0 when nothing drives them) and prints the groups that rise together.

Only one line is ever an output, so nothing can fight: wiring two pins
together is never a short. 3.3 V logic, a few mA at most: don't connect it to
anything powered.

    ad3_continuity.py                       all 16 lines, named DIO0..DIO15
    ad3_continuity.py 0=tip 1=ring 2=red    only these lines, with names

WaveForms is unpacked in /opt/digilent (not installed with dpkg), so this
re-runs itself with the library path set.
"""

import ctypes
import os
import sys
import time

LIBRARY_PATH = "/opt/digilent/usr/lib/digilent/adept:/opt/digilent/usr/lib"
SETTLE = 0.05  # s: the AD3 reads back a driven line wrong at 10 ms

if LIBRARY_PATH not in os.environ.get("LD_LIBRARY_PATH", ""):
    os.environ["LD_LIBRARY_PATH"] = LIBRARY_PATH
    os.execv(sys.executable, [sys.executable] + sys.argv)

dwf = ctypes.cdll.LoadLibrary("libdwf.so.3")


def open_device():
    handle = ctypes.c_int()
    dwf.FDwfDeviceOpen(ctypes.c_int(-1), ctypes.byref(handle))
    if handle.value == 0:
        message = ctypes.create_string_buffer(512)
        dwf.FDwfGetLastErrorMsg(message)
        sys.exit(f"can't open the AD3: {message.value.decode()}")
    # On a port that only advertises default USB power the AD3 limits itself
    # to 0.6 W and shuts its instruments down, yet every call still reports
    # success and the inputs read back stale values. Running, it draws ~0.6 A
    dwf.FDwfAnalogIOStatus(handle)
    usb_amps = ctypes.c_double()
    dwf.FDwfAnalogIOChannelNodeStatus(handle, ctypes.c_int(2), ctypes.c_int(3), ctypes.byref(usb_amps))
    if usb_amps.value < 0.3:
        dwf.FDwfDeviceClose(handle)
        sys.exit(f"AD3 draws {usb_amps.value:.2f} A: shut down for lack of USB power. Use a "
                 "USB-C port offering 1.5-3 A, or set 'USB Limit: Maximum' in the WaveForms GUI")
    dwf.FDwfDigitalIOReset(handle)
    return handle


def read_inputs(handle):
    dwf.FDwfDigitalIOStatus(handle)
    state = ctypes.c_uint()
    dwf.FDwfDigitalIOInputStatus(handle, ctypes.byref(state))
    return state.value


def drive(handle, line):
    """`line` high and every other line an input (None: all inputs)"""
    mask = 0 if line is None else 1 << line
    dwf.FDwfDigitalIOOutputEnableSet(handle, ctypes.c_uint(mask))
    dwf.FDwfDigitalIOOutputSet(handle, ctypes.c_uint(mask))
    dwf.FDwfDigitalIOConfigure(handle)
    time.sleep(SETTLE)


def main():
    names = {}
    for arg in sys.argv[1:]:
        line, _, name = arg.partition("=")
        names[int(line)] = name or f"DIO{line}"
    if not names:
        names = {line: f"DIO{line}" for line in range(16)}
    lines = sorted(names)
    used = sum(1 << line for line in lines)

    handle = open_device()
    try:
        # Nothing driven: anything already high is powered from outside
        drive(handle, None)
        idle = read_inputs(handle) & used
        if idle:
            stuck = [names[line] for line in lines if idle >> line & 1]
            sys.exit(f"high with nothing driven, something is powered: {stuck}")

        follows = {}
        for line in lines:
            drive(handle, line)
            state = read_inputs(handle) & used
            follows[line] = {other for other in lines if other != line and state >> other & 1}
            # A driven line that doesn't read back high is shorted to ground
            if not state >> line & 1:
                print(f"WARNING: {names[line]} driven high reads low: tied to GND?")
        drive(handle, None)
    finally:
        dwf.FDwfDeviceClose(handle)

    # Nets: a line joins the net of everything it pulled high
    seen = set()
    for line in lines:
        if line in seen:
            continue
        net = {line} | follows[line]
        seen |= net
        members = ", ".join(names[member] for member in sorted(net))
        print(members if len(net) > 1 else f"{members} (alone)")
    # Asymmetric pairs mean a diode or something odd, not a wire
    for line in lines:
        for other in follows[line]:
            if line not in follows[other]:
                print(f"WARNING: {names[line]} pulls {names[other]} up but not back")


if __name__ == "__main__":
    main()
