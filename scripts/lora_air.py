"""What our radio settings mean on the air, for the scripts.

Reads the LoRa and packet constants straight from the firmware's source
(core/src/air.rs, core/src/codec.rs, core/src/packet.rs, src/devices/radio.rs),
so a changed constant changes every script's numbers. The formulas are
core/src/air.rs's, line for line (Semtech's air-time formula); its tests pin
the results (a voice packet is 65.792 ms at SF7/125k).

Used by air-diagram.py and relay-test.py.
"""

import math
import os
import re

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


def _source(path):
    with open(os.path.join(ROOT, path)) as f:
        return f.read()


def _const(source, name):
    m = re.search(rf"pub const {name}: \w+ = ([\d_]+);", source)
    if not m:
        raise SystemExit(f"lora_air: no `pub const {name}` found: did the source change?")
    return int(m.group(1).replace("_", ""))


_air = _source("core/src/air.rs")
_codec = _source("core/src/codec.rs")

SPREADING_FACTOR = _const(_air, "SPREADING_FACTOR")
BANDWIDTH_KHZ = _const(_air, "BANDWIDTH_KHZ")
CODING_RATE = _const(_air, "CODING_RATE")  # 4/(4+this)
PREAMBLE_SYMBOLS = _const(_air, "PREAMBLE_SYMBOLS")

CODEC2_FRAME_BYTES = _const(_codec, "CODEC2_FRAME_BYTES")
CODEC2_FRAME_SAMPLES = _const(_codec, "CODEC2_FRAME_SAMPLES")
FRAMES_PER_PACKET = _const(_codec, "FRAMES_PER_PACKET")
HEADER_BYTES = _const(_codec, "HEADER_BYTES")
PACKET_BYTES = HEADER_BYTES + CODEC2_FRAME_BYTES * FRAMES_PER_PACKET
SAMPLE_RATE_HZ = 8000  # Codec2's

# Our 2-byte header's fields, from packet.rs's doc comment: |5b type|7b txid|3b hops|1b spare|
HEADER_FIELDS = [
    (int(bits), name)
    for bits, name in re.findall(r"(\d+)b (\w+)", re.search(r"//! .*?(\|.*\|)", _source("core/src/packet.rs")).group(1))
]

# Symbols per CAD while sweeping (radio.rs CAD_SETTINGS: CADSymbols::_N)
CAD_SYMBOLS = int(re.search(r"CAD_SETTINGS.*CADSymbols::_(\d+)", _source("src/devices/radio.rs")).group(1))

SYNC_SYMBOLS = 4.25  # 2 sync word + 2.25 start of frame
HEADER_BLOCK_SYMBOLS = 8  # the PHY header's block, always at CR 4/8


def symbol_ms():
    """2^SF / bandwidth: 1.024 ms at SF7/125k"""
    return (1 << SPREADING_FACTOR) / BANDWIDTH_KHZ


def low_data_rate():
    """Low data rate optimisation: on for symbols over 16 ms (SF11+ at 125k)"""
    return symbol_ms() > 16


def bits_per_block():
    """Data bits per payload block: 4 x (SF - 2DE)"""
    return 4 * (SPREADING_FACTOR - 2 * low_data_rate())


def symbols_per_block():
    return CODING_RATE + 4


def payload_blocks(n_bytes):
    """Payload blocks after the header block, explicit header, and a 16-bit
    CRC (ours, core/src/crc.rs, with LoRa's off: the same bits on the air)"""
    bits = 8 * n_bytes - 4 * SPREADING_FACTOR + 28 + 16
    return max(0, math.ceil(bits / bits_per_block()))


def payload_symbols(n_bytes):
    """Symbols after the sync word: the header block, then the payload blocks"""
    return HEADER_BLOCK_SYMBOLS + payload_blocks(n_bytes) * symbols_per_block()


def packet_symbols(n_bytes, preamble=PREAMBLE_SYMBOLS):
    return preamble + SYNC_SYMBOLS + payload_symbols(n_bytes)


def packet_ms(n_bytes, preamble=PREAMBLE_SYMBOLS):
    return packet_symbols(n_bytes, preamble) * symbol_ms()


def wake_preamble_symbols():
    """As long as it can be while a header-only packet still takes a voice
    packet's air (air::wake_preamble_symbols)"""
    return PREAMBLE_SYMBOLS + payload_symbols(PACKET_BYTES) - payload_symbols(HEADER_BYTES)


def frame_ms():
    return CODEC2_FRAME_SAMPLES * 1000 / SAMPLE_RATE_HZ


def slot_ms():
    """One packet's audio, and so the time from one packet to the next"""
    return FRAMES_PER_PACKET * frame_ms()


# A voice packet's air time and the packet period, in ms
AIR_MS = packet_ms(PACKET_BYTES)
PERIOD_MS = slot_ms()
