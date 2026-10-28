#!/usr/bin/env python3
"""Record AD3 scope channel 1 for a fixed time and save it as .npz (volts, rate, start time).

  ad3_record.py <output.npz> [seconds] [samples_per_second] [centre_volts] [vplus_volts]

vplus_volts turns on the AD3's V+ supply for the recording (off afterwards): a clean bias
source, e.g. for an electret mic, where a Jumperless DAC adds ~12 mV of 33 Hz steps.

Uses the 5 V range (+-2.5 V around centre_volts, ~0.3 mV steps), so a mic line sitting at
~1.1 V DC still shows millivolt speech. The start time (epoch seconds) is saved so the trace
can be lined up with a bench_record.py video.

WaveForms is unpacked in /opt/digilent (not installed with dpkg), so this re-runs itself
with the library path set.
"""
import ctypes
import os
import sys
import time

LIBRARY_PATH = "/opt/digilent/usr/lib/digilent/adept:/opt/digilent/usr/lib"
if LIBRARY_PATH not in os.environ.get("LD_LIBRARY_PATH", ""):
    os.environ["LD_LIBRARY_PATH"] = LIBRARY_PATH
    os.execv(sys.executable, [sys.executable] + sys.argv)

import numpy as np  # noqa: E402

dwf = ctypes.cdll.LoadLibrary("libdwf.so.3")
# From dwf.h
PARAM_USB_LIMIT = 6
ACQUISITION_MODE_RECORD = 3
STATE_DONE = 2

output = sys.argv[1]
seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 15
samples_per_second = float(sys.argv[3]) if len(sys.argv) > 3 else 20000
centre_volts = float(sys.argv[4]) if len(sys.argv) > 4 else 0.0
vplus_volts = float(sys.argv[5]) if len(sys.argv) > 5 else None

# On a port that only advertises default USB power the AD3 limits itself to 0.6 W and shuts
# its instruments down, while every call still reports success. Lifting the limit before
# opening wakes it (~0.68 A running, ~0.12 A asleep).
dwf.FDwfParamSet(ctypes.c_int(PARAM_USB_LIMIT), ctypes.c_int(-1))
handle = ctypes.c_int()
dwf.FDwfDeviceOpen(ctypes.c_int(-1), ctypes.byref(handle))
if handle.value == 0:
    message = ctypes.create_string_buffer(512)
    dwf.FDwfGetLastErrorMsg(message)
    sys.exit(f"can't open the AD3: {message.value.decode()}")
time.sleep(0.5)
dwf.FDwfAnalogIOStatus(handle)
usb_amps = ctypes.c_double()
dwf.FDwfAnalogIOChannelNodeStatus(handle, ctypes.c_int(2), ctypes.c_int(3), ctypes.byref(usb_amps))
if usb_amps.value < 0.3:
    dwf.FDwfDeviceClose(handle)
    sys.exit(f"AD3 draws {usb_amps.value:.2f} A: asleep for lack of USB power")

if vplus_volts is not None:
    # AnalogIO channel 0 = V+ (node 0 enable, node 1 volts); the master switch turns it on.
    dwf.FDwfAnalogIOChannelNodeSet(handle, ctypes.c_int(0), ctypes.c_int(1), ctypes.c_double(vplus_volts))
    dwf.FDwfAnalogIOChannelNodeSet(handle, ctypes.c_int(0), ctypes.c_int(0), ctypes.c_double(1))
    dwf.FDwfAnalogIOEnableSet(handle, ctypes.c_int(1))
    dwf.FDwfAnalogIOConfigure(handle)
    time.sleep(1)

channel = ctypes.c_int(0)
dwf.FDwfAnalogInChannelEnableSet(handle, channel, ctypes.c_int(1))
dwf.FDwfAnalogInChannelRangeSet(handle, channel, ctypes.c_double(5.0))
dwf.FDwfAnalogInChannelOffsetSet(handle, channel, ctypes.c_double(centre_volts))
dwf.FDwfAnalogInAcquisitionModeSet(handle, ctypes.c_int(ACQUISITION_MODE_RECORD))
dwf.FDwfAnalogInFrequencySet(handle, ctypes.c_double(samples_per_second))
dwf.FDwfAnalogInRecordLengthSet(handle, ctypes.c_double(seconds))
# Let the input settle after setting range and offset.
dwf.FDwfAnalogInConfigure(handle, ctypes.c_int(1), ctypes.c_int(0))
time.sleep(1)

chunks = []
lost_total = corrupt_total = 0
dwf.FDwfAnalogInConfigure(handle, ctypes.c_int(0), ctypes.c_int(1))
start = time.time()
state = ctypes.c_byte()
available, lost, corrupt = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
while True:
    dwf.FDwfAnalogInStatus(handle, ctypes.c_int(1), ctypes.byref(state))
    dwf.FDwfAnalogInStatusRecord(handle, ctypes.byref(available), ctypes.byref(lost), ctypes.byref(corrupt))
    lost_total += lost.value
    corrupt_total += corrupt.value
    if available.value:
        buffer = (ctypes.c_double * available.value)()
        dwf.FDwfAnalogInStatusData(handle, channel, buffer, available)
        chunks.append(np.frombuffer(buffer, dtype=np.float64).copy())
    if state.value == STATE_DONE and not available.value:
        break
    time.sleep(0.01)
if vplus_volts is not None:
    dwf.FDwfAnalogIOEnableSet(handle, ctypes.c_int(0))
    dwf.FDwfAnalogIOConfigure(handle)
dwf.FDwfDeviceClose(handle)

volts = np.concatenate(chunks) if chunks else np.zeros(0)
np.savez(output, volts=volts, samples_per_second=samples_per_second, start=start)
print(f"{len(volts)} samples ({len(volts) / samples_per_second:.1f} s), lost {lost_total}, corrupt {corrupt_total}, "
      f"min {volts.min():.4f} V, max {volts.max():.4f} V, mean {volts.mean():.4f} V -> {output}")
