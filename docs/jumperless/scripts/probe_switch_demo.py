# Respond to probe switch: Measure mode shows ADC voltage; Select mode shows touched pad.

import time

while True:
    position = check_switch_position()
    if position == SWITCH_MEASURE:
        voltage = get_adc(0)
        oled_print("Voltage: {}V".format(voltage))
        print("Voltage: {}V".format(voltage))
    elif position == SWITCH_SELECT:
        pad = probe_read(blocking=False)
        if pad != NO_PAD:
            oled_print("Touched: {}".format(pad))
            print("Touched: {}".format(pad))
    time.sleep(0.1)
