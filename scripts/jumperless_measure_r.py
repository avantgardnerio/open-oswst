"""Measure the resistance between two breadboard rows with the Jumperless.

  jumperless_measure_r.py <session> <name> --pair 56-58 [--pair 55-58 ...] [--volts 0.5,1.0]

For each pair A-B: bottom rail -> current sensor (I+ -> I-) -> row A, row B -> GND, ADC0 on A,
ADC1 on B. R = (V_A - V_B) / I. Rail voltages are capped at 2 V.

The V5's current sensor (ina_get_current(0)) reads a resting current even with I- routed
nowhere: about 0.77 mA per volt on I-, i.e. a ~1.3 kOhm load inside the board, in ~30 uA
steps (measured 2026-10-27). So every run first fits that baseline on an empty row and
subtracts it at the voltage row A actually sits at. Without it a 5.1 kOhm resistor read
1.3 kOhm.

A row already above 0.3 V before anything is driven stops the run (something live on it).
Non-ohmic parts (an electret mic's FET, diodes) show up as R changing with voltage: that is
why --volts takes a list.
"""
import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import bench  # noqa: E402

MAX_VOLTS = 2.0

DEVICE_CODE = """
import time

PAIRS = {pairs}
VOLTS = {volts}
EMPTY_ROW = {empty_row}
IDLE_LIMIT_VOLTS = 0.3


def avg(read, count=16):
    return sum(read() for _ in range(count)) / count


def settle_and_read():
    time.sleep(0.3)
    return avg(lambda: ina_get_current(0)), avg(lambda: adc_get(0)), avg(lambda: adc_get(1))


overlay_clear_all()
dac_set(BOTTOM_RAIL, 0.0, False)

# Baseline: the sensor's own resting current, as a straight line in the I- voltage.
connect(ISENSE_MINUS, EMPTY_ROW)
connect(ADC0, EMPTY_ROW)
points = []
for volts in VOLTS + [max(VOLTS) * 0.5]:
    dac_set(BOTTOM_RAIL, volts, False)
    amps, row_volts, _ = settle_and_read()
    points.append((row_volts, amps))
dac_set(BOTTOM_RAIL, 0.0, False)
disconnect(ISENSE_MINUS, EMPTY_ROW)
disconnect(ADC0, EMPTY_ROW)
count = len(points)
mean_volts = sum(point[0] for point in points) / count
mean_amps = sum(point[1] for point in points) / count
slope = sum((point[0] - mean_volts) * (point[1] - mean_amps) for point in points) / sum((point[0] - mean_volts) ** 2 for point in points)
offset = mean_amps - slope * mean_volts
print("BASELINE %.4f mA/V %+.4f mA" % (slope * 1000, offset * 1000))

for row_a, row_b in PAIRS:
    connect(ADC0, row_a)
    connect(ADC1, row_b)
    time.sleep(0.2)
    idle_a, idle_b = avg(lambda: adc_get(0)), avg(lambda: adc_get(1))
    if max(abs(idle_a), abs(idle_b)) > IDLE_LIMIT_VOLTS:
        print("ABORT %d-%d already driven: %.3f V, %.3f V" % (row_a, row_b, idle_a, idle_b))
    else:
        connect(ISENSE_MINUS, row_a)
        connect(row_b, GND)
        for volts in VOLTS:
            dac_set(BOTTOM_RAIL, volts, False)
            amps, volts_a, volts_b = settle_and_read()
            through = amps - (slope * volts_a + offset)
            ohms = (volts_a - volts_b) / through if through > 1e-6 else float("inf")
            print("PAIR %d-%d rail %.2f V: I %.4f mA (part %.4f mA), A %.4f V, B %.4f V -> R %.1f ohm" % (row_a, row_b, volts, amps * 1000, through * 1000, volts_a, volts_b, ohms))
        dac_set(BOTTOM_RAIL, 0.0, False)
        disconnect(ISENSE_MINUS, row_a)
        disconnect(row_b, GND)
    disconnect(ADC0, row_a)
    disconnect(ADC1, row_b)
print("DONE rail", dac_get(BOTTOM_RAIL))
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("session")
    parser.add_argument("name")
    parser.add_argument("--pair", action="append", required=True, help="A-B: current flows into A, B is grounded")
    parser.add_argument("--volts", default="0.5,1.0", help="rail voltages, comma separated, max 2 V")
    parser.add_argument("--empty-row", type=int, default=45, help="a row with nothing in it, for the baseline")
    args = parser.parse_args()

    pairs = [tuple(int(row) for row in pair.split("-")) for pair in args.pair]
    volts = [float(value) for value in args.volts.split(",")]
    if max(volts) > MAX_VOLTS:
        sys.exit(f"rail capped at {MAX_VOLTS} V")
    if any(args.empty_row in pair for pair in pairs):
        sys.exit("the baseline row must not be one of the measured rows")

    bench.log(args.session, f"Resistance **{args.name}**: pairs {', '.join(args.pair)}, rail {args.volts} V, baseline row {args.empty_row}")
    stdout, stderr = bench.run(DEVICE_CODE.format(pairs=pairs, volts=volts, empty_row=args.empty_row), 60 + 10 * len(pairs) * len(volts))
    print(stdout, end="")
    if stderr:
        print("STDERR:", stderr)
    bench.log(args.session, f"```\n{stdout}{stderr}```")


if __name__ == "__main__":
    main()
